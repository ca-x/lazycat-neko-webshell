use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{BoxFuture, InvokeReply, PluginError, SessionContext, TargetRequest};

/// A host builds the table; each plugin receives a copy containing only its grants.
/// This is an API boundary for trusted native code, not a native-code sandbox.
#[derive(Clone, Default)]
pub struct Resources {
    grants: HashSet<String>,
    services: HashMap<String, Arc<dyn Any + Send + Sync>>,
}

impl std::fmt::Debug for Resources {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Resources")
            .field("grants", &self.grants)
            .field("service_count", &self.services.len())
            .finish_non_exhaustive()
    }
}

impl Resources {
    pub fn provide<T: Send + Sync + 'static>(&mut self, name: &str, service: T) {
        self.services.insert(name.to_owned(), Arc::new(service));
    }

    #[must_use]
    /// Copy only the services named by this plugin's capability grants.
    pub fn scoped(&self, grants: &[String]) -> Self {
        Self {
            grants: grants.iter().cloned().collect(),
            services: self
                .services
                .iter()
                .filter(|(key, _)| grants.contains(key))
                .map(|(key, value)| (key.clone(), Arc::clone(value)))
                .collect(),
        }
    }

    /// Retrieve a shared host service with the requested concrete type.
    ///
    /// # Errors
    /// Returns permission denied for an ungranted name, or unavailable when the
    /// service is missing or has a different concrete type.
    pub fn require<T: Send + Sync + 'static>(&self, name: &str) -> Result<Arc<T>, PluginError> {
        if !self.grants.contains(name) {
            return Err(PluginError::permission_denied(format!(
                "plugin resource was not granted: {name}"
            )));
        }
        self.services
            .get(name)
            .and_then(|service| Arc::clone(service).downcast::<T>().ok())
            .ok_or_else(|| {
                PluginError::unavailable(format!("plugin resource is unavailable: {name}"))
            })
    }
}

/// Canonical identity returned by the host, used to bind transient file operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetIdentity {
    pub session_id: String,
    pub selector: String,
    pub login_user: String,
}
impl TargetIdentity {
    #[must_use]
    /// Encode all identity fields as an unambiguous transient-upload owner key.
    pub fn upload_owner(&self) -> String {
        serde_json::json!([&self.selector, &self.login_user, &self.session_id]).to_string()
    }
}

#[derive(Debug)]
pub enum FileOperation {
    List { path: String },
    Stat { path: String },
    Read { path: String, content_type: String },
    Write { path: String, data: Vec<u8> },
}

pub trait TargetFiles: Send + Sync {
    /// Resolve the host-authorized target, retaining its complete identity.
    ///
    /// # Errors
    /// Returns an error for a missing session, target mismatch, or failed authorization.
    fn authorize<'a>(
        &'a self,
        target: &'a TargetRequest,
    ) -> BoxFuture<'a, Result<TargetIdentity, PluginError>>;
    /// Execute a file operation after revalidating the target's authorization.
    ///
    /// # Errors
    /// Returns a validation, authorization, transport, or target filesystem error.
    fn execute<'a>(
        &'a self,
        target: &'a TargetRequest,
        operation: FileOperation,
    ) -> BoxFuture<'a, Result<InvokeReply, PluginError>>;
}

pub trait TargetContext: Send + Sync {
    /// Resolve an existing or target-local session with its current authorization.
    ///
    /// # Errors
    /// Returns an error for an invalid, unavailable, or unauthorized target.
    fn resolve<'a>(
        &'a self,
        target: &'a TargetRequest,
    ) -> BoxFuture<'a, Result<SessionContext, PluginError>>;
}

/// The host binds a namespace and maps legacy persisted keys before supplying this.
pub trait PluginStorage: Send + Sync {
    /// Load a value from this plugin's host-bound namespace.
    ///
    /// # Errors
    /// Returns the host's storage or decoding error.
    fn load(&self, key: &str) -> io::Result<Option<Vec<u8>>>;
    /// Persist a value in this plugin's host-bound namespace.
    ///
    /// # Errors
    /// Returns the host's storage error without reporting a successful write.
    fn store(&self, key: &str, value: &[u8]) -> io::Result<()>;
}

pub trait AiPreferences: Send + Sync {
    /// Read the host-owned AI configuration for this plugin.
    ///
    /// # Errors
    /// Returns an I/O error when the configuration cannot be loaded.
    fn load(&self) -> io::Result<Vec<u8>>;
}

pub trait TunnelProfiles: Send + Sync {
    /// Return profile metadata with credentials removed.
    ///
    /// # Errors
    /// Returns a host storage or profile decoding error.
    fn public_metadata(&self) -> Result<HashMap<String, String>, PluginError>;
    /// Validate and replace the saved tunnel profiles.
    ///
    /// # Errors
    /// Returns a validation or persistence error.
    fn replace(&self, profiles: &str) -> Result<(), PluginError>;
    /// Retrieve the token of an authorized ngrok profile.
    ///
    /// # Errors
    /// Returns an error for a missing profile, token, or unreadable store.
    fn ngrok_token(&self, profile_id: &str) -> Result<String, PluginError>;
    /// Record successful use of a tunnel profile.
    ///
    /// # Errors
    /// Returns an error if the profile cannot be found or persisted.
    fn mark_used(&self, profile_id: &str) -> Result<(), PluginError>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebshellNotification {
    pub id: String,
    pub source_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub kind: String,
    pub severity: String,
    #[serde(default = "default_presentation_hint")]
    pub presentation_hint: String,
    pub title: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub actions: Vec<NotificationAction>,
    pub state: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

fn default_presentation_hint() -> String {
    "center".to_owned()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub payload: Value,
}

#[derive(Clone, Debug)]
pub struct NewNotification {
    pub source_kind: String,
    pub source_id: Option<String>,
    pub kind: String,
    pub severity: String,
    pub presentation_hint: String,
    pub title: String,
    pub body: String,
    pub url: Option<String>,
    pub actions: Vec<NotificationAction>,
}

pub trait PluginNotifications: Send + Sync {
    /// Add a notification scoped to this plugin.
    ///
    /// # Errors
    /// Returns a host persistence or notification validation error.
    fn add(&self, input: NewNotification) -> io::Result<WebshellNotification>;
    /// Look up a notification visible to this plugin.
    ///
    /// # Errors
    /// Returns a host persistence or decoding error.
    fn notification(&self, id: &str) -> io::Result<Option<WebshellNotification>>;
    /// Dismiss a visible notification, returning its updated state if present.
    ///
    /// # Errors
    /// Returns a host persistence error.
    fn dismiss(&self, id: &str) -> io::Result<Option<WebshellNotification>>;
    /// Mark a visible notification's action as handled.
    ///
    /// # Errors
    /// Returns a host persistence error.
    fn mark_actioned(&self, id: &str) -> io::Result<Option<WebshellNotification>>;
}

#[must_use]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().try_into().unwrap_or(u64::MAX)
        })
}

pub trait LightOsForward: Send + Sync {
    /// Invoke forwarding against an authorized physical target.
    ///
    /// # Errors
    /// Returns a validation, authorization, or forwarding process error.
    fn invoke<'a>(
        &'a self,
        target: &'a TargetRequest,
        operation: &'a str,
    ) -> BoxFuture<'a, Result<InvokeReply, PluginError>>;
    /// Stop this plugin's active forwarding processes.
    ///
    /// # Errors
    /// Returns a host error if forwarding state cannot be locked or cleaned up.
    fn stop_all(&self) -> Result<(), PluginError>;
}

/// Persistent directory granted by the host to one ASR plugin only.
/// Native plugins remain trusted code; this resource is an ownership boundary.
#[derive(Clone, Debug)]
pub struct AsrModelDirectory {
    pub path: std::path::PathBuf,
}
