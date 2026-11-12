use super::store::{Grant, RootIdentity};
use crate::state::AppState;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
    time::timeout,
};

pub struct Helper {
    child: Option<Child>,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    pub identity: RootIdentity,
    poisoned: bool,
}
impl Helper {
    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }
    pub async fn start(state: &Arc<AppState>, grant: &Grant, read_only: bool) -> Result<Self> {
        let root = grant.root.as_deref().context("未授权文件目录")?;
        ensure!(
            root.starts_with('/') && !root.contains('\0') && root.len() <= 4096,
            "共享目录必须是绝对路径"
        );
        let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
        let script = format!(
            "exec python3 -u -c {} {} {}",
            quote(include_str!("files_helper.py")),
            quote(root),
            if read_only { "read-only" } else { "operate" }
        );
        let mut command = target_command(state, grant, &script, read_only)?;
        command
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .context("启动目标文件服务失败（需要 Python3）")?;
        let input = child.stdin.take().context("target stdin missing")?;
        let output = BufReader::new(child.stdout.take().context("target stdout missing")?);
        let mut helper = Self {
            child: Some(child),
            input: Some(input),
            output,
            identity: RootIdentity {
                device: 0,
                inode: 0,
                uid: 0,
            },
            poisoned: false,
        };
        let ready = helper.line().await?;
        helper.identity =
            serde_json::from_value(ready["ready"].clone()).context("目标文件服务未就绪")?;
        if let Some(expected) = &grant.root_identity {
            ensure!(
                expected.device == helper.identity.device
                    && expected.inode == helper.identity.inode
                    && expected.uid == helper.identity.uid,
                "共享根目录身份已变化，请重新授权"
            );
        }
        Ok(helper)
    }
    async fn line(&mut self) -> Result<Value> {
        let mut b = Vec::new();
        let n = timeout(
            Duration::from_secs(15),
            (&mut self.output).take(65537).read_until(b'\n', &mut b),
        )
        .await
        .context("目标文件服务响应超时")??;
        ensure!(
            n > 0 && b.len() <= 65536 && b.last() == Some(&b'\n'),
            "目标文件服务协议错误"
        );
        Ok(serde_json::from_slice(&b)?)
    }
    pub async fn call(&mut self, value: Value) -> Result<Value> {
        ensure!(!self.poisoned, "目标文件服务连接已失效，请重新开始传输");
        let mut b = serde_json::to_vec(&value)?;
        ensure!(b.len() < 65536, "文件请求过大");
        b.push(b'\n');
        self.poisoned = true;
        let reply = async {
            timeout(
                Duration::from_secs(15),
                self.input
                    .as_mut()
                    .context("目标文件服务已关闭")?
                    .write_all(&b),
            )
            .await
            .context("目标文件服务写入超时")??;
            let reply = self.line().await?;
            ensure!(
                reply.is_object() && (reply.get("ok").is_some() != reply.get("error").is_some()),
                "目标文件服务返回无效结果"
            );
            Ok::<_, anyhow::Error>(reply)
        }
        .await;
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => {
                self.close().await;
                return Err(error);
            }
        };
        self.poisoned = false;
        ensure!(
            reply.get("error").is_none(),
            "{}",
            reply["message"].as_str().unwrap_or("目标文件操作失败")
        );
        reply.get("ok").cloned().context("目标文件服务返回无效结果")
    }
    pub async fn close(&mut self) {
        self.poisoned = true;
        self.input.take();
        if let Some(mut child) = self.child.take()
            && timeout(Duration::from_secs(2), child.wait()).await.is_err()
        {
            let _ = child.kill().await;
        }
    }
}
impl Drop for Helper {
    fn drop(&mut self) {
        self.input.take();
        if let Some(mut child) = self.child.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(async move {
                if timeout(Duration::from_secs(2), child.wait()).await.is_err() {
                    let _ = child.kill().await;
                }
            });
        }
    }
}

fn target_command(
    state: &Arc<AppState>,
    grant: &Grant,
    script: &str,
    read_only: bool,
) -> Result<tokio::process::Command> {
    #[cfg(not(test))]
    let _ = read_only;
    #[cfg(test)]
    if let Some(command) = super::peer::test_target::file_command(state, grant, read_only) {
        return Ok(command);
    }
    let command = if crate::ssh_backend::is_ssh_selector(&grant.selector) {
        let p = crate::ssh_backend::load_enabled_profile(&state.database(), &grant.selector)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        crate::ssh_backend::profile_script_command(&p, script, false)
    } else {
        ensure!(!grant.login.is_empty(), "目标登录身份缺失");
        let bootstrap = crate::plugin::resources::files::script_for_session_user(
            &crate::state::SessionRecord {
                id: grant.session.clone(),
                host: String::new(),
                selector: grant.selector.clone(),
                status: String::new(),
                cols: 80,
                rows: 24,
                command: String::new(),
                args: vec![],
                metadata: std::collections::HashMap::from([(
                    "loginUser".into(),
                    grant.login.clone(),
                )]),
            },
            script,
        );
        let mut c = tokio::process::Command::new(crate::config::LIGHTOSCTL);
        c.args(["exec", "-i", &grant.selector, "/bin/sh", "-c", &bootstrap]);
        c
    };
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scripted(script: &str) -> Helper {
        let mut child = tokio::process::Command::new("/bin/sh")
            .args(["-c", script])
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        Helper {
            input: child.stdin.take(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child: Some(child),
            identity: RootIdentity {
                device: 0,
                inode: 0,
                uid: 0,
            },
            poisoned: false,
        }
    }

    #[tokio::test]
    async fn malformed_response_poisons_and_closes_helper() {
        for script in [
            "printf 'invalid-json\\n'; cat >/dev/null",
            "printf '{\"unexpected\":true}\\n'; cat >/dev/null",
            "printf '{\"ok\":true}'; exit 0",
        ] {
            let mut helper = scripted(script);
            assert!(helper.call(json!({"action":"list"})).await.is_err());
            assert!(helper.is_poisoned());
            assert!(helper.child.is_none());
            assert!(helper.call(json!({"action":"list"})).await.is_err());
        }
    }

    #[tokio::test]
    async fn valid_operation_error_preserves_framing() {
        let mut helper = scripted(
            "printf '{\"error\":\"ValueError\",\"message\":\"invalid path\"}\\n{\"ok\":true}\\n'; cat >/dev/null",
        );
        assert!(helper.call(json!({"action":"list"})).await.is_err());
        assert!(!helper.is_poisoned());
        assert_eq!(
            helper.call(json!({"action":"list"})).await.unwrap(),
            json!(true)
        );
        helper.close().await;
    }
}
