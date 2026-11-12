//! Current-user executable and a host-owned namespace. No discovery starts a core.
use anyhow::{Context, Result, ensure};
use neko_hc_client::Client;
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Clone, Debug)]
pub(super) struct Launch {
    pub binary: PathBuf,
    pub root: PathBuf,
    pub socket: PathBuf,
}
impl Launch {
    pub fn discover(
        namespace: &str,
        program: Option<&Path>,
        root: Option<&Path>,
    ) -> Result<Option<Self>> {
        let binary = if let Some(program) = program {
            Some(program.to_owned())
        } else {
            find_program()?
        };
        let Some(binary) = binary else {
            return Ok(None);
        };
        validate_binary(&binary)?;
        let root = if let Some(root) = root {
            root.to_owned()
        } else {
            let base = std::env::var_os("XDG_STATE_HOME")
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .or_else(|| {
                    if cfg!(windows) {
                        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
                    } else {
                        std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state"))
                    }
                })
                .context("hc requires a current-user state directory")?;
            base.join("neoko-terminal/hc").join(namespace)
        };
        ensure!(root.is_absolute(), "hc state directory must be absolute");
        #[cfg(unix)]
        let socket = PathBuf::from("/tmp")
            .join(format!(
                "nkhc-{}",
                uuid::Uuid::parse_str(namespace)?.simple()
            ))
            .join("core.sock");
        #[cfg(windows)]
        let socket = PathBuf::from(format!(
            r"\\.\pipe\neoko-terminal-hc-{}",
            uuid::Uuid::parse_str(namespace)?.simple()
        ));
        Ok(Some(Self {
            binary,
            root,
            socket,
        }))
    }
    pub fn prepare(&self, namespace: &str) -> Result<()> {
        private_dir(&self.root)?;
        private_dir(&self.root.join("config"))?;
        #[cfg(unix)]
        private_dir(self.socket.parent().context("hc socket has no parent")?)?;
        let marker = self.root.join("owner");
        use std::io::Write as _;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
        {
            Ok(mut file) => {
                file.write_all(namespace.as_bytes())?;
                file.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                ensure!(
                    !std::fs::symlink_metadata(&marker)?.file_type().is_symlink()
                        && std::fs::read_to_string(&marker)? == namespace,
                    "hc state namespace ownership changed"
                );
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }
    pub async fn connect(&self, namespace: &str, start: bool) -> Result<Client> {
        validate_binary(&self.binary)?;
        self.prepare(namespace)?;
        if !start {
            self.validate_socket().await?;
        }
        let mut command = Command::new(&self.binary);
        // Per-child changes only. Keep the user's HOME and shell, isolate hc's own stores.
        command
            .arg("connect")
            .env_remove("HC_TERMINAL_ID")
            .env_remove("HC_CORE_ID")
            .env("HC_SESSION", "neoko-terminal")
            .env("HC_SOCKET", &self.socket)
            .env("XDG_STATE_HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"));
        if let Some(home) = std::env::var_os("HOME").filter(|p| !p.is_empty()) {
            command.current_dir(home);
        }
        let client = Client::start(command).await?;
        self.validate_socket().await?;
        Ok(client)
    }
    #[cfg(unix)]
    async fn validate_socket(&self) -> Result<()> {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let metadata = std::fs::symlink_metadata(&self.socket)
            .context("hc is disconnected; use Connect hc explicitly")?;
        ensure!(
            metadata.file_type().is_socket()
                && metadata.uid() == rustix::process::geteuid().as_raw()
                && metadata.mode() & 0o022 == 0,
            "hc socket is not private to the current user"
        );
        Ok(())
    }
    #[cfg(windows)]
    async fn validate_socket(&self) -> Result<()> {
        crate::herdr::transport::verify_core_pipe(&self.socket)
    }
}
#[cfg(unix)]
fn private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    if !path.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
    }
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.mode() & 0o077 == 0,
        "hc directory is not private to the current user"
    );
    Ok(())
}
#[cfg(windows)]
fn private_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    ensure!(
        !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
        "hc state directory is a link"
    );
    Ok(())
}
fn validate_binary(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute(),
        "hc executable must have an absolute path"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(path)?;
        ensure!(
            m.is_file()
                && m.mode() & 0o111 != 0
                && m.mode() & 0o022 == 0
                && (m.uid() == 0 || m.uid() == rustix::process::geteuid().as_raw()),
            "hc executable has unsafe ownership or permissions"
        );
    }
    #[cfg(windows)]
    ensure!(path.is_file(), "hc executable is unavailable");
    Ok(())
}
fn find_program() -> Result<Option<PathBuf>> {
    // Test hosts never discover a real user's installation implicitly.
    #[cfg(test)]
    {
        Ok(None)
    }
    #[cfg(not(test))]
    {
        for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .filter(|p| p.is_absolute())
            .take(64)
        {
            let file = dir.join(if cfg!(windows) { "hc.exe" } else { "hc" });
            if file.is_file() {
                return Ok(Some(file.canonicalize()?));
            }
        }
        Ok(None)
    }
}
