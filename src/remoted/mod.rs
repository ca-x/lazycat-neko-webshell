//! Application-owned remote access. Disabled unless explicitly configured by the deployment owner.
mod admission;
mod authority;
mod files;
mod heartbeat;
pub(crate) mod http;
mod peer;
pub(crate) mod relay;
mod store;
mod terminal_creation;
mod terminals;
pub(crate) use store::Remoted;
