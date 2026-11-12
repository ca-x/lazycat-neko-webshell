//! Platform transports verify that the discovered endpoint belongs to this user.
use anyhow::{Result, ensure};
use std::path::Path;

#[cfg(unix)]
pub(super) fn validate_binary(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path)?;
    let uid = rustix::process::geteuid().as_raw();
    ensure!(
        metadata.is_file()
            && metadata.mode() & 0o111 != 0
            && metadata.mode() & 0o022 == 0
            && (metadata.uid() == uid || metadata.uid() == 0),
        "Herdr executable is not owned by this user/root or is writable by others"
    );
    Ok(())
}
#[cfg(windows)]
pub(super) fn validate_binary(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute() && path.is_file(),
        "Herdr executable is unavailable"
    );
    Ok(())
}
#[cfg(unix)]
pub(super) async fn connect(path: &Path) -> Result<tokio::net::UnixStream> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let metadata = std::fs::symlink_metadata(path)?;
    let uid = rustix::process::geteuid().as_raw();
    ensure!(
        metadata.file_type().is_socket() && metadata.uid() == uid && metadata.mode() & 0o022 == 0,
        "Herdr socket is not a private current-user socket"
    );
    let stream = tokio::net::UnixStream::connect(path).await?;
    ensure!(
        stream.peer_cred()?.uid() == uid,
        "Herdr peer is not the current user"
    );
    Ok(stream)
}
#[cfg(windows)]
pub(super) async fn connect(
    path: &Path,
) -> Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    use std::os::windows::io::AsRawHandle;
    ensure!(
        std::fs::metadata(path)?.len() <= 4096,
        "Herdr pipe marker exceeds limit"
    );
    let marker = std::fs::read_to_string(path)?;
    let name = marker
        .trim()
        .strip_prefix('@')
        .ok_or_else(|| anyhow::anyhow!("Herdr pipe marker is invalid"))?;
    ensure!(
        name.starts_with("herdr-sockapi-")
            && name.len() <= 240
            && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'),
        "Herdr pipe name is not local"
    );
    let stream =
        tokio::net::windows::named_pipe::ClientOptions::new().open(format!(r"\\.\pipe\{name}"))?;
    // SAFETY: the live NamedPipeClient owns this valid pipe handle for the entire check.
    unsafe {
        windows_identity::verify(stream.as_raw_handle())?;
    }
    Ok(stream)
}
#[cfg(windows)]
mod windows_identity {
    use super::*;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    use windows_sys::Win32::{
        Foundation::HANDLE,
        Security::{EqualSid, GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser},
        System::{
            Pipes::GetNamedPipeServerProcessId,
            Threading::{
                GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
    };

    // SAFETY: callers provide a live local pipe. All created process/token handles are owned.
    pub(super) unsafe fn verify(pipe: RawHandle) -> Result<()> {
        let mut pid = 0;
        ensure!(
            unsafe { GetNamedPipeServerProcessId(pipe as HANDLE, &mut pid) } != 0,
            "cannot inspect Herdr pipe owner"
        );
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        ensure!(!process.is_null(), "cannot inspect Herdr process");
        let process = unsafe { OwnedHandle::from_raw_handle(process as RawHandle) };
        let server = unsafe { user(process.as_raw_handle() as HANDLE) }?;
        let current = unsafe { user(GetCurrentProcess()) }?;
        let server_user = unsafe { &*(server.as_ptr() as *const TOKEN_USER) };
        let current_user = unsafe { &*(current.as_ptr() as *const TOKEN_USER) };
        ensure!(
            unsafe { EqualSid(server_user.User.Sid, current_user.User.Sid) } != 0,
            "Herdr pipe server is not the current user"
        );
        Ok(())
    }
    // SAFETY: process is valid. The aligned returned allocation retains its embedded SID.
    unsafe fn user(process: HANDLE) -> Result<Vec<usize>> {
        let mut token = std::ptr::null_mut();
        ensure!(
            unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } != 0,
            "cannot inspect Herdr user token"
        );
        let token = unsafe { OwnedHandle::from_raw_handle(token as RawHandle) };
        let mut length = 0;
        unsafe {
            GetTokenInformation(
                token.as_raw_handle() as HANDLE,
                TokenUser,
                std::ptr::null_mut(),
                0,
                &mut length,
            );
        }
        ensure!(length > 0 && length <= 16384, "invalid Herdr user token");
        let mut bytes = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
        ensure!(
            unsafe {
                GetTokenInformation(
                    token.as_raw_handle() as HANDLE,
                    TokenUser,
                    bytes.as_mut_ptr().cast(),
                    length,
                    &mut length,
                )
            } != 0,
            "cannot read Herdr user token"
        );
        Ok(bytes)
    }
}

#[cfg(unix)]
pub(super) fn identity(path: &Path) -> Result<String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path)?;
    Ok(format!(
        "{}:{}:{}:{}",
        metadata.dev(),
        metadata.ino(),
        metadata.ctime(),
        metadata.ctime_nsec()
    ))
}
#[cfg(windows)]
pub(super) fn identity(path: &Path) -> Result<String> {
    ensure!(
        std::fs::metadata(path)?.len() <= 4096,
        "Herdr pipe marker exceeds limit"
    );
    Ok(std::fs::read_to_string(path)?)
}

#[cfg(windows)]
pub(crate) fn verify_core_pipe(path: &Path) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    let stream = tokio::net::windows::named_pipe::ClientOptions::new().open(path)?;
    // SAFETY: the client retains its valid pipe handle for the identity check.
    unsafe { windows_identity::verify(stream.as_raw_handle()) }
}
