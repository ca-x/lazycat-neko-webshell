//! Launch installed target hc without borrowing another login or core's environment.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, bail, ensure};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::config::LIGHTOSCTL;
use crate::validation::validate_selector;

const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
const PROBE_OUTPUT_LIMIT: usize = 16 * 1024;
const SYSTEM_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Prepare a non-PTY connection to an installed hc in a provider-owned scope.
///
/// # Errors
/// Rejects invalid selectors, empty login names, and scopes other than lowercase hex32.
/// Target identity, binary and directory checks run only when the command is spawned.
pub fn command(selector: &str, login_user: &str, scope: &str) -> anyhow::Result<Command> {
    ensure!(
        scope.len() == 32
            && scope
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "hc scope must be 32 lowercase hexadecimal characters"
    );
    target_command(selector, login_user, &user_script(Some(scope)))
}

/// Check installed executable paths without running hc or creating directories.
///
/// # Errors
/// Invalid identity, transport failure, unexpected output, timeout, or excessive output
/// are errors. A successful probe with no installed executable returns false.
pub async fn available(selector: &str, login_user: &str) -> anyhow::Result<bool> {
    probe(target_command(selector, login_user, &user_script(None))?).await
}

fn target_command(selector: &str, login_user: &str, script: &str) -> anyhow::Result<Command> {
    validate_selector(selector).map_err(|err| anyhow::anyhow!("invalid hc selector: {err}"))?;
    ensure!(
        !login_user.trim().is_empty() && !login_user.contains('\0'),
        "hc login user is required"
    );
    let mut command = Command::new(LIGHTOSCTL);
    command.args([
        "exec",
        "-i",
        selector,
        "/bin/sh",
        "-c",
        &bootstrap(login_user, script),
    ]);
    command.kill_on_drop(true);
    Ok(command)
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn bootstrap(login_user: &str, script: &str) -> String {
    // Clear the environment before identity resolution as well as after switching users.
    let body = format!(
        r#"set -eu
user={user}
case "$user" in -*) exit 126 ;; esac
entry=$(getent passwd "$user") || exit 126
IFS=: read -r name password uid gid gecos home shell <<NEKO_PASSWD
$entry
NEKO_PASSWD
[ "$name" = "$user" ] || exit 126
case "$uid" in ''|*[!0-9]*) exit 126 ;; esac
case "$gid" in ''|*[!0-9]*) exit 126 ;; esac
case "$home" in /*) ;; *) exit 126 ;; esac
run_script={run_script}
if command -v setpriv >/dev/null 2>&1; then
  exec setpriv --reuid "$uid" --regid "$gid" --init-groups /bin/sh -c "$run_script" hc-launch "$home" "$name" "$uid" "$gid"
fi
if command -v su >/dev/null 2>&1; then
  exec su -s /bin/sh -c "$run_script" -- "$name" hc-launch "$home" "$name" "$uid" "$gid"
fi
echo 'hc requires setpriv or su' >&2
exit 126"#,
        user = quote(login_user),
        run_script = quote(&login_script(script)),
    );
    format!(
        "PATH={}\nexport PATH\nexec env -i PATH=\"$PATH\" /bin/sh -c {}",
        quote(SYSTEM_PATH),
        quote(&body)
    )
}

fn login_script(script: &str) -> String {
    format!(
        r#"exec env -i HOME="$1" USER="$2" LOGNAME="$2" PATH="{SYSTEM_PATH}" XDG_CONFIG_HOME="$1/.config" XDG_DATA_HOME="$1/.local/share" XDG_STATE_HOME="$1/.local/state" XDG_CACHE_HOME="$1/.cache" /bin/sh -c {script} hc-user "$3" "$4""#,
        script = quote(script)
    )
}

fn user_script(scope: Option<&str>) -> String {
    let action = match scope {
        Some(scope) => format!(
            r#"binary=$(find_binary) || {{ echo 'hc is not installed for the target login' >&2; exit 127; }}
runtime="/tmp/neko-hc-$uid-{scope}"
umask 077
if [ ! -e "$runtime" ] && [ ! -L "$runtime" ]; then
  mkdir -m 700 -- "$runtime" 2>/dev/null || [ -d "$runtime" ] || exit 126
fi
[ ! -L "$runtime" ] && [ -d "$runtime" ] || {{ echo 'unsafe hc runtime directory' >&2; exit 126; }}
[ "$(stat -c '%u:%a' -- "$runtime")" = "$uid:700" ] || {{ echo 'unsafe hc runtime ownership or permissions' >&2; exit 126; }}
export HC_SESSION='neko-{scope}' HC_SOCKET="$runtime/core.sock" XDG_RUNTIME_DIR="$runtime"
cd "$HOME"
export PATH="$HOME/.local/bin:$HOME/bin:$PATH"
exec "$binary" connect"#
        ),
        None => r"if find_binary >/dev/null; then
  printf 'NEKO_HC_AVAILABLE\n'
else
  printf 'NEKO_HC_MISSING\n'
fi"
        .to_owned(),
    };
    format!(
        r#"set -eu
uid="$1"
gid="$2"
[ "$(id -u)" = "$uid" ] && [ "$(id -g)" = "$gid" ] || exit 126
find_binary() {{
  for dir in "$HOME/.local/bin" "$HOME/bin" /usr/local/bin /usr/bin; do
    for name in hc herdr-core; do
      if [ -f "$dir/$name" ] && [ -x "$dir/$name" ]; then
        printf '%s\n' "$dir/$name"
        return 0
      fi
    done
  done
  return 1
}}
{action}"#
    )
}

async fn probe(mut command: Command) -> anyhow::Result<bool> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().context("starting hc availability probe")?;
    let result = tokio::time::timeout(PROBE_TIMEOUT, async {
        let mut stdout = child
            .stdout
            .take()
            .context("hc probe stdout is not piped")?;
        let mut stderr = child
            .stderr
            .take()
            .context("hc probe stderr is not piped")?;
        let mut output = Vec::new();
        let mut total = 0;
        let mut out_open = true;
        let mut err_open = true;
        let mut out_buf = [0; 1024];
        let mut err_buf = [0; 1024];
        while out_open || err_open {
            let (count, is_stdout) = tokio::select! {
                count = stdout.read(&mut out_buf), if out_open => (count?, true),
                count = stderr.read(&mut err_buf), if err_open => (count?, false),
            };
            total += count;
            ensure!(
                total <= PROBE_OUTPUT_LIMIT,
                "hc availability probe output exceeded 16 KiB"
            );
            if is_stdout {
                out_open = count != 0;
                output.extend_from_slice(&out_buf[..count]);
            } else {
                err_open = count != 0;
            }
        }
        ensure!(
            child.wait().await?.success(),
            "hc availability probe failed"
        );
        match output.as_slice() {
            b"NEKO_HC_AVAILABLE\n" => Ok(true),
            b"NEKO_HC_MISSING\n" => Ok(false),
            _ => bail!("unexpected hc availability probe response"),
        }
    })
    .await;
    let result = result
        .context("hc availability probe timed out after 8 seconds")
        .and_then(|result| result);
    if result.is_err() {
        let _ = child.kill().await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    const SCOPE: &str = "0123456789abcdef0123456789abcdef";

    struct Fixture {
        home: PathBuf,
        scope: String,
        uid: String,
        gid: String,
    }

    impl Fixture {
        fn new() -> Self {
            let scope = uuid::Uuid::new_v4().simple().to_string();
            let home = std::env::temp_dir().join(format!("neko-hc-test-{scope}"));
            fs::create_dir_all(home.join(".local/bin")).unwrap();
            Self {
                home,
                scope,
                uid: identity("-u"),
                gid: identity("-g"),
            }
        }

        fn runtime(&self) -> PathBuf {
            PathBuf::from(format!("/tmp/neko-hc-{}-{}", self.uid, self.scope))
        }

        fn install(&self, script: &str) {
            let binary = self.home.join(".local/bin/hc");
            fs::write(&binary, format!("#!/bin/sh\n{script}\n")).unwrap();
            fs::set_permissions(binary, fs::Permissions::from_mode(0o700)).unwrap();
        }

        fn launch(&self) -> std::process::Output {
            // Exercise the exact post-switch command as the test's existing UID/GID.
            std::process::Command::new("/bin/sh")
                .args([
                    "-c",
                    &login_script(&user_script(Some(&self.scope))),
                    "hc-launch",
                ])
                .arg(&self.home)
                .args(["test-login", &self.uid, &self.gid])
                .env("HC_SOCKET", "/unrelated/core.sock")
                .env("HC_SESSION", "other")
                .env("HC_TERMINAL_ID", "other-terminal")
                .env("HC_TERMINAL_SOCKET", "/unrelated/terminal.sock")
                .env("HC_TERMINAL_FUTURE", "must-not-leak")
                .env("XDG_RUNTIME_DIR", "/unrelated")
                .env("UNTRUSTED", "must-not-leak")
                .output()
                .unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let runtime = self.runtime();
            if runtime.is_symlink() {
                let _ = fs::remove_file(runtime);
            } else {
                let _ = fs::remove_dir_all(runtime);
            }
            let _ = fs::remove_dir_all(&self.home);
        }
    }

    fn identity(flag: &str) -> String {
        let output = std::process::Command::new("id").arg(flag).output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn validates_identity_and_keeps_lightos_transport_non_pty() {
        for scope in ["", "../bad", "0123456789ABCDEF0123456789ABCDEF", "0123"] {
            assert!(command("target@owner", "login", scope).is_err());
        }
        assert!(command("bad", "login", SCOPE).is_err());
        assert!(command("target@owner", " ", SCOPE).is_err());
        let selector = "target;echo injected@owner";
        let command = command(selector, "a'b;$(echo injected)", SCOPE).unwrap();
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert_eq!(&args[..5], ["exec", "-i", selector, "/bin/sh", "-c"]);
        assert_eq!(args.len(), 6);
        assert_eq!(command.as_std().get_program(), LIGHTOSCTL);
        assert!(!args.contains(&"-t"));
        assert!(!args.contains(&"-it"));
    }

    #[test]
    fn shell_quote_round_trips_metacharacters_and_newlines() {
        for value in ["a'b;$(touch nope)", "x\n`echo injected`", "", "-root"] {
            let output = std::process::Command::new("/bin/sh")
                .args(["-c", &format!("printf '%s' {}", quote(value))])
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(output.stdout, value.as_bytes());
        }
        let script = bootstrap(
            "no-such-login'; echo NEKO_INJECTION; #",
            "echo SHOULD_NOT_RUN",
        );
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &script])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }

    #[test]
    fn bootstrap_does_not_resolve_root_tools_from_inherited_path() {
        let fixture = Fixture::new();
        let fake_env = fixture.home.join(".local/bin/env");
        fs::write(&fake_env, "#!/bin/sh\necho INHERITED_PATH_EXECUTED\n").unwrap();
        fs::set_permissions(&fake_env, fs::Permissions::from_mode(0o700)).unwrap();
        let output = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                &bootstrap("no-such-neko-login", "echo SHOULD_NOT_RUN"),
            ])
            .env("PATH", fixture.home.join(".local/bin"))
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "inherited PATH executed before environment reset"
        );
        assert!(output.stdout.is_empty());
    }

    #[test]
    fn connect_uses_clean_environment_and_private_stable_scope() {
        let fixture = Fixture::new();
        fixture.install("printf '%s\\n' \"$1\"; env | sort");
        let before_home = std::env::var_os("HOME");
        let before_cwd = std::env::current_dir().unwrap();
        let first = fixture.launch();
        assert!(
            first.status.success(),
            "{}",
            String::from_utf8_lossy(&first.stderr)
        );
        let text = String::from_utf8(first.stdout).unwrap();
        assert!(text.starts_with("connect\n"));
        for expected in [
            format!("HOME={}\n", fixture.home.display()),
            "USER=test-login\n".into(),
            "LOGNAME=test-login\n".into(),
            format!("HC_SESSION=neko-{}\n", fixture.scope),
            format!("HC_SOCKET={}/core.sock\n", fixture.runtime().display()),
            format!("XDG_RUNTIME_DIR={}\n", fixture.runtime().display()),
            format!("XDG_STATE_HOME={}/.local/state\n", fixture.home.display()),
            format!(
                "PATH={}/.local/bin:{}/bin:{SYSTEM_PATH}\n",
                fixture.home.display(),
                fixture.home.display()
            ),
        ] {
            assert!(text.contains(&expected), "missing {expected:?}: {text}");
        }
        assert!(!text.contains("HC_TERMINAL"));
        assert!(!text.contains("UNTRUSTED"));
        assert!(!text.contains("/unrelated"));
        let second = fixture.launch();
        assert!(second.status.success());
        assert_eq!(
            fs::metadata(fixture.runtime())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(std::env::var_os("HOME"), before_home);
        assert_eq!(std::env::current_dir().unwrap(), before_cwd);
    }

    #[test]
    fn rejects_runtime_symlink_and_open_permissions() {
        let fixture = Fixture::new();
        fixture.install("echo CONNECTED");
        std::os::unix::fs::symlink(&fixture.home, fixture.runtime()).unwrap();
        assert!(!fixture.launch().status.success());
        fs::remove_file(fixture.runtime()).unwrap();
        fs::create_dir(fixture.runtime()).unwrap();
        fs::set_permissions(fixture.runtime(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!fixture.launch().status.success());
        // Ownership is compared to the resolved UID, never to a caller-supplied identity.
        let script = user_script(Some(&fixture.scope));
        assert!(script.contains("stat -c '%u:%a'"));
        assert!(script.contains("= \"$uid:700\""));
        assert!(bootstrap("login", &script).contains("getent passwd"));
        assert!(bootstrap("login", &script).contains("--init-groups"));
    }

    #[test]
    fn missing_binary_cannot_report_connect_success_and_probe_is_read_only() {
        let fixture = Fixture::new();
        // Hide system search paths so host installation cannot affect this fixture.
        let connect = user_script(Some(&fixture.scope))
            .replace("/usr/local/bin /usr/bin", "/nonexistent-neko-hc-test");
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &login_script(&connect), "hc-launch"])
            .arg(&fixture.home)
            .args(["test-login", &fixture.uid, &fixture.gid])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(127));
        assert!(!fixture.runtime().exists());
        let probe_script =
            user_script(None).replace("/usr/local/bin /usr/bin", "/nonexistent-neko-hc-test");
        let mut probe = std::process::Command::new("/bin/sh");
        probe
            .args(["-c", &probe_script, "hc-probe", &fixture.uid, &fixture.gid])
            .env("HOME", &fixture.home);
        let missing = probe.output().unwrap();
        assert!(missing.status.success());
        assert_eq!(missing.stdout, b"NEKO_HC_MISSING\n");
        fixture.install("echo HC_MUST_NOT_BE_EXECUTED; exit 99");
        let binary = fixture.home.join(".local/bin/hc");
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(probe.output().unwrap().stdout, b"NEKO_HC_MISSING\n");
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let found = probe.output().unwrap();
        assert!(found.status.success());
        assert_eq!(found.stdout, b"NEKO_HC_AVAILABLE\n");
        assert!(!fixture.runtime().exists());
    }

    #[test]
    fn accepts_alternate_installed_binary_name_under_user_bin() {
        let fixture = Fixture::new();
        fixture.install("printf '%s' \"$1\"");
        fs::create_dir(fixture.home.join("bin")).unwrap();
        fs::rename(
            fixture.home.join(".local/bin/hc"),
            fixture.home.join("bin/herdr-core"),
        )
        .unwrap();
        let output = fixture.launch();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"connect");
    }

    fn shell(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }

    #[tokio::test]
    async fn probe_checks_exit_and_exact_marker_and_combined_output_bound() {
        assert!(probe(shell("printf 'NEKO_HC_AVAILABLE\\n'")).await.unwrap());
        assert!(!probe(shell("printf 'NEKO_HC_MISSING\\n'")).await.unwrap());
        assert!(
            probe(shell("printf 'NEKO_HC_AVAILABLE\\n'; exit 7"))
                .await
                .is_err()
        );
        assert!(probe(shell("echo unexpected")).await.is_err());
        let stderr_budget = PROBE_OUTPUT_LIMIT - b"NEKO_HC_AVAILABLE\n".len();
        assert!(
            probe(shell(&format!(
                "printf 'NEKO_HC_AVAILABLE\\n'; head -c {stderr_budget} /dev/zero >&2"
            )))
            .await
            .unwrap()
        );
        assert!(
            probe(shell("head -c 9000 /dev/zero; head -c 9000 /dev/zero >&2"))
                .await
                .unwrap_err()
                .to_string()
                .contains("16 KiB")
        );
        assert!(
            probe(shell("exec yes"))
                .await
                .unwrap_err()
                .to_string()
                .contains("16 KiB")
        );
        assert!(
            probe(shell("exec yes >&2"))
                .await
                .unwrap_err()
                .to_string()
                .contains("16 KiB")
        );
    }

    #[tokio::test]
    async fn probe_has_eight_second_deadline() {
        let result = probe(shell("exec sleep 30")).await;
        assert!(result.unwrap_err().to_string().contains("8 seconds"));
    }
}
