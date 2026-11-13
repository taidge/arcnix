use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuthMethod {
    Password,
    PrivateKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerProfile {
    pub id: String,
    pub name: String,
    pub group_id: Option<String>,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthMethod,
    pub private_key: Option<PathBuf>,
    pub initial_remote_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_pane_ratio: Option<u16>,
}

impl Default for ServerProfile {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: String::new(),
            group_id: None,
            host: String::new(),
            port: 22,
            username: String::new(),
            auth: AuthMethod::Password,
            private_key: None,
            initial_remote_path: ".".into(),
            file_pane_ratio: None,
        }
    }
}

pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{:x}-{:x}-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerGroup {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustedHost {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AppConfig {
    pub groups: Vec<ServerGroup>,
    pub servers: Vec<ServerProfile>,
    pub trusted_hosts: Vec<TrustedHost>,
    pub local_directory: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transfer_rules: Vec<TransferRule>,
    #[serde(default)]
    pub settings: AppSettings,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    English,
    SimplifiedChinese,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

impl Theme {
    pub fn is_dark(self, system_dark: bool) -> bool {
        match self {
            Self::System => system_dark,
            Self::Dark => true,
            Self::Light => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSettings {
    pub language: Language,
    #[serde(default)]
    pub theme: Theme,
    pub show_hidden_files: bool,
    pub resource_interval_seconds: u64,
}
impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: Language::English,
            theme: Theme::System,
            show_hidden_files: true,
            resource_interval_seconds: 3,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExistingFileAction {
    Overwrite,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransferRule {
    pub server_id: String,
    pub direction: TransferDirection,
    pub action: ExistingFileAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<u64>,
    pub permissions: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum TransferDirection {
    Upload,
    Download,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferStatus {
    Queued,
    Running,
    Completed,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct TransferJob {
    pub id: String,
    pub server_id: String,
    pub direction: TransferDirection,
    pub local_path: PathBuf,
    pub remote_path: String,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub status: TransferStatus,
}

#[derive(Debug, Clone, Default)]
pub struct ServerInfo {
    pub hostname: String,
    pub os: String,
    pub uptime: String,
    pub memory: String,
    pub disk: String,
}

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}
