//! Local and encrypted remote workspaces share typed application messages.
use super::*;

pub(super) enum WorkspaceChannel {
    Remote(Box<Session>),
    #[cfg(not(target_os = "android"))]
    Local(neoko_terminal_host::LocalSession),
}
impl From<Session> for WorkspaceChannel {
    fn from(session: Session) -> Self {
        Self::Remote(Box::new(session))
    }
}
impl WorkspaceChannel {
    pub async fn request(&mut self, operation: Operation) -> Result<u64> {
        match self {
            Self::Remote(session) => session.request(operation).await,
            #[cfg(not(target_os = "android"))]
            Self::Local(session) => session.request(operation).await,
        }
    }
    pub async fn receive(&mut self) -> Result<Response> {
        match self {
            Self::Remote(session) => session.receive().await,
            #[cfg(not(target_os = "android"))]
            Self::Local(session) => session.receive().await,
        }
    }
    pub fn host_key(&self) -> Result<&[u8; 32]> {
        match self {
            Self::Remote(session) => Ok(session.host_key()),
            #[cfg(not(target_os = "android"))]
            Self::Local(_) => bail!("本机工作区不接受远程配对消息"),
        }
    }
    pub fn connection_kind(&self) -> protocol::messages::ConnectionKind {
        match self {
            Self::Remote(session) => session.connection_kind(),
            #[cfg(not(target_os = "android"))]
            Self::Local(_) => protocol::messages::ConnectionKind::Local,
        }
    }
    pub async fn enable_direct(&mut self) -> Result<()> {
        match self {
            Self::Remote(session) => session.enable_direct().await,
            #[cfg(not(target_os = "android"))]
            Self::Local(_) => Ok(()),
        }
    }
    pub async fn close(self) {
        match self {
            Self::Remote(session) => (*session).close().await,
            #[cfg(not(target_os = "android"))]
            Self::Local(session) => session.close().await,
        }
    }
}

/// A stopped host notification stream is disabled rather than polled in a loop.
pub(super) async fn host_changed<T: Clone>(receiver: &mut Option<tokio::sync::watch::Receiver<T>>) {
    if let Some(updates) = receiver {
        if updates.changed().await.is_ok() {
            return;
        }
        *receiver = None;
    }
    std::future::pending::<()>().await;
}
