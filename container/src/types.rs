use prost_types::Timestamp;
use std::collections::HashMap;
use std::time::SystemTime;

pub fn now_timestamp() -> Timestamp {
    Timestamp::from(SystemTime::now())
}

#[derive(Debug, Clone, Default)]
pub struct ContainerConfig {
    pub hostname: String,
    pub domainname: String,
    pub user: String,
    pub attach_stdin: bool,
    pub attach_stdout: bool,
    pub attach_stderr: bool,
    pub exposed_ports: Vec<String>,
    pub tty: bool,
    pub open_stdin: bool,
    pub stdin_once: bool,
    pub env: Vec<String>,
    pub cmd: Vec<String>,
    pub entrypoint: Vec<String>,
    pub image: String,
    pub labels: HashMap<String, String>,
    pub working_dir: String,
    pub network_disabled: bool,
    pub mac_address: String,
    pub stop_signal: String,
    pub stop_timeout: i32,
    pub shell: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct HostConfig {
    pub binds: Vec<String>,
    pub network_mode: String,
    pub port_bindings: Vec<PortBinding>,
    pub auto_remove: bool,
    pub privileged: bool,
    pub publish_all_ports: bool,
    pub readonly_rootfs: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PortBinding {
    pub host_ip: String,
    pub host_port: String,
    pub container_port: String,
    pub protocol: String,
}

#[derive(Debug, Clone, Default)]
pub struct NetworkingConfig {
    pub endpoints_config: HashMap<String, EndpointConfig>,
}

#[derive(Debug, Clone, Default)]
pub struct EndpointConfig {
    pub network_id: String,
    pub endpoint_id: String,
    pub gateway: String,
    pub ip_address: String,
    pub ip_prefix_len: i32,
    pub mac_address: String,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CreateContainerParams {
    pub name: Option<String>,
    pub config: ContainerConfig,
    pub host_config: HostConfig,
    pub networking_config: NetworkingConfig,
}

#[derive(Debug, Clone)]
pub struct CreateContainerResult {
    pub id: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ListContainersParams {
    pub all: bool,
    pub limit: i32,
    pub size: bool,
    pub filters: HashMap<String, String>,
}

#[derive(Debug, Clone, Default)]
pub struct Container {
    pub id: String,
    pub names: Vec<String>,
    pub image: String,
    pub image_id: String,
    pub command: String,
    pub created: Option<Timestamp>,
    pub state: String,
    pub status: String,
    pub ports: Vec<PortBinding>,
    pub labels: HashMap<String, String>,
    pub size_rw: i64,
    pub size_root_fs: i64,
}

#[derive(Debug, Clone, Default)]
pub struct ContainerState {
    pub status: String,
    pub running: bool,
    pub paused: bool,
    pub restarting: bool,
    pub oom_killed: bool,
    pub dead: bool,
    pub pid: i32,
    pub exit_code: i32,
    pub error: String,
    pub started_at: Option<Timestamp>,
    pub finished_at: Option<Timestamp>,
}

#[derive(Debug, Clone)]
pub struct ContainerInspection {
    pub container: Container,
    pub state: ContainerState,
    pub path: String,
    pub args: Vec<String>,
    pub resolv_conf_path: String,
    pub hostname_path: String,
    pub hosts_path: String,
    pub log_path: String,
    pub name: String,
    pub restart_count: i64,
    pub driver: String,
    pub platform: String,
    pub mount_label: String,
    pub process_label: String,
    pub app_armor_profile: String,
    pub exec_ids: Vec<String>,
    pub config: ContainerConfig,
    pub host_config: HostConfig,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: Timestamp,
    pub stream: String,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct GetLogsParams {
    pub container_id: String,
    pub follow: bool,
    pub stdout: bool,
    pub stderr: bool,
    pub since: Option<Timestamp>,
    pub until: Option<Timestamp>,
    pub timestamps: bool,
    pub tail: String,
}

#[derive(Debug, Clone, Default)]
pub struct ExecConfig {
    pub attach_stdin: bool,
    pub attach_stdout: bool,
    pub attach_stderr: bool,
    pub detach_keys: String,
    pub tty: bool,
    pub env: Vec<String>,
    pub cmd: Vec<String>,
    pub privileged: bool,
    pub user: String,
    pub working_dir: String,
}

#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub stream: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct AttachInput {
    pub container_id: String,
    pub stream: bool,
    pub stdin: bool,
    pub stdout: bool,
    pub stderr: bool,
    pub detach_keys: String,
    pub logs: bool,
    pub input: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct AttachOutput {
    pub stream: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct WaitResult {
    pub status_code: i64,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum OutputEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Exit(WaitResult),
}

#[derive(Debug, Clone, Default)]
pub struct StatsParams {
    pub container_id: String,
    pub stream: bool,
    pub one_shot: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ContainerStats {
    pub read: Option<Timestamp>,
    pub preread: Option<Timestamp>,
    pub num_procs: u32,
    pub cpu_stats: Option<CpuStats>,
    pub precpu_stats: Option<CpuStats>,
    pub memory_stats: Option<MemoryStats>,
    pub networks: HashMap<String, NetworkStats>,
}

#[derive(Debug, Clone, Default)]
pub struct CpuStats {
    pub cpu_usage: Option<CpuUsage>,
    pub system_cpu_usage: u64,
    pub online_cpus: u64,
}

#[derive(Debug, Clone, Default)]
pub struct CpuUsage {
    pub total_usage: u64,
    pub percpu_usage: Vec<u64>,
    pub usage_in_kernelmode: u64,
    pub usage_in_usermode: u64,
}

#[derive(Debug, Clone, Default)]
pub struct MemoryStats {
    pub usage: u64,
    pub max_usage: u64,
    pub stats: HashMap<String, u64>,
    pub failcnt: u64,
    pub limit: u64,
    pub commit: u64,
    pub commit_peak: u64,
    pub private_working_set: u64,
}

#[derive(Debug, Clone, Default)]
pub struct NetworkStats {
    pub rx_bytes: u64,
    pub rx_packets: u64,
    pub rx_errors: u64,
    pub rx_dropped: u64,
    pub tx_bytes: u64,
    pub tx_packets: u64,
    pub tx_errors: u64,
    pub tx_dropped: u64,
}

#[derive(Debug, Clone)]
pub enum InputEvent {
    Stdin(Vec<u8>),
    Resize { width: u16, height: u16 },
}
