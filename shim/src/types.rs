use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContainerConfig {
    pub image: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub env: Vec<String>,
    pub cmd: Vec<String>,
    pub entrypoint: Vec<String>,
    pub working_dir: Option<String>,
    pub labels: HashMap<String, String>,
    pub tty: bool,
    pub open_stdin: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostConfig {
    pub binds: Vec<String>,
    pub network_mode: Option<String>,
    pub privileged: bool,
    pub readonly_rootfs: bool,
    pub auto_remove: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerState {
    Created,
    Running,
    Paused,
    Stopped,
}

impl std::fmt::Display for ContainerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerState::Created => write!(f, "created"),
            ContainerState::Running => write!(f, "running"),
            ContainerState::Paused => write!(f, "paused"),
            ContainerState::Stopped => write!(f, "stopped"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub id: String,
    pub name: Option<String>,
    pub image: String,
    pub state: ContainerState,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub bundle_path: String,
    pub rootfs_path: String,
}

#[derive(Debug, Clone)]
pub struct CreateContainerOpts {
    pub name: Option<String>,
    pub config: ContainerConfig,
    pub host_config: HostConfig,
    pub mounts: Vec<SnapshotMount>,
}

#[derive(Debug, Clone)]
pub struct SnapshotMount {
    pub mount_type: String,
    pub source: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct WaitResult {
    pub exit_code: i32,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum OutputEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Exit(WaitResult),
}
