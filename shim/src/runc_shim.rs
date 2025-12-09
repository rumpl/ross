use crate::error::ShimError;
use crate::types::*;
use futures::Stream;
use oci_spec::runtime::{
    LinuxBuilder, LinuxNamespace, LinuxNamespaceBuilder, LinuxNamespaceType, Mount, MountBuilder,
    ProcessBuilder, RootBuilder, Spec, SpecBuilder,
};
use ross_mount::MountSpec;
use runc::options::{DeleteOpts, GlobalOpts, KillOpts};
use runc::Runc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContainerMetadata {
    info: ContainerInfo,
    config: ContainerConfig,
    host_config: HostConfig,
}

pub struct RuncShim {
    runc: Runc,
    data_dir: PathBuf,
    containers: Arc<RwLock<HashMap<String, ContainerMetadata>>>,
}

impl RuncShim {
    pub async fn new(data_dir: &Path) -> Result<Self, ShimError> {
        let containers_dir = data_dir.join("containers");
        fs::create_dir_all(&containers_dir).await?;

        let runc = GlobalOpts::new()
            .root(data_dir.join("runc"))
            .debug(true)
            .log(data_dir.join("runc.log"))
            .build()
            .map_err(|e| ShimError::Runc(e.to_string()))?;

        let shim = Self {
            runc,
            data_dir: data_dir.to_path_buf(),
            containers: Arc::new(RwLock::new(HashMap::new())),
        };

        shim.load_containers().await?;

        Ok(shim)
    }

    async fn load_containers(&self) -> Result<(), ShimError> {
        let containers_dir = self.data_dir.join("containers");
        let mut entries = fs::read_dir(&containers_dir).await?;
        let mut containers = self.containers.write().await;

        while let Some(entry) = entries.next_entry().await? {
            let metadata_path = entry.path().join("metadata.json");
            if metadata_path.exists() {
                let content = fs::read_to_string(&metadata_path).await?;
                if let Ok(metadata) = serde_json::from_str::<ContainerMetadata>(&content) {
                    containers.insert(metadata.info.id.clone(), metadata);
                }
            }
        }

        Ok(())
    }

    async fn save_container(&self, metadata: &ContainerMetadata) -> Result<(), ShimError> {
        let container_dir = self.data_dir.join("containers").join(&metadata.info.id);
        fs::create_dir_all(&container_dir).await?;
        let metadata_path = container_dir.join("metadata.json");
        let content = serde_json::to_string_pretty(metadata)?;
        fs::write(&metadata_path, content).await?;
        Ok(())
    }

    pub async fn create(&self, opts: CreateContainerOpts) -> Result<String, ShimError> {
        let id = Uuid::new_v4().to_string();

        {
            let containers = self.containers.read().await;
            if containers.contains_key(&id) {
                return Err(ShimError::ContainerAlreadyExists(id));
            }
        }

        let bundle_path = self.data_dir.join("containers").join(&id).join("bundle");
        let rootfs_path = bundle_path.join("rootfs");
        fs::create_dir_all(&bundle_path).await?;
        fs::create_dir_all(&rootfs_path).await?;

        // Mount the rootfs using the snapshotter mount specification
        self.mount_rootfs(&opts.mounts, &rootfs_path).await?;

        let spec = self.generate_spec(&opts, &rootfs_path)?;
        tracing::info!("Generated OCI spec with args: {:?}", spec.process().as_ref().and_then(|p| p.args().as_ref()));
        let spec_path = bundle_path.join("config.json");
        let spec_content = serde_json::to_string_pretty(&spec)?;
        tracing::debug!("OCI spec content: {}", &spec_content);
        fs::write(&spec_path, spec_content).await?;

        // Create log files for stdout/stderr
        let stdout_path = bundle_path.join("stdout.log");
        let stderr_path = bundle_path.join("stderr.log");
        fs::write(&stdout_path, "").await?;
        fs::write(&stderr_path, "").await?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let info = ContainerInfo {
            id: id.clone(),
            name: opts.name.clone(),
            image: opts.config.image.clone(),
            state: ContainerState::Created,
            pid: None,
            exit_code: None,
            created_at: now,
            started_at: None,
            finished_at: None,
            bundle_path: bundle_path.to_string_lossy().to_string(),
            rootfs_path: rootfs_path.to_string_lossy().to_string(),
        };

        let metadata = ContainerMetadata {
            info,
            config: opts.config,
            host_config: opts.host_config,
        };

        self.save_container(&metadata).await?;

        {
            let mut containers = self.containers.write().await;
            containers.insert(id.clone(), metadata);
        }

        tracing::info!(container_id = %id, "Container created (bundle prepared)");
        Ok(id)
    }

    async fn mount_rootfs(
        &self,
        mounts: &[SnapshotMount],
        target: &Path,
    ) -> Result<(), ShimError> {
        if mounts.is_empty() {
            return Err(ShimError::Runc("No mounts provided".to_string()));
        }

        let mount = &mounts[0];
        tracing::info!(
            "Mounting rootfs: type={}, source={}, options={:?}",
            mount.mount_type,
            mount.source,
            mount.options
        );
        
        let spec = MountSpec::new(&mount.mount_type, &mount.source, mount.options.clone());

        ross_mount::mount_overlay(&spec, target)
            .map_err(|e| ShimError::Runc(format!("Failed to mount rootfs: {}", e)))?;

        Ok(())
    }

    pub async fn start(&self, id: &str) -> Result<(), ShimError> {
        let bundle_path: PathBuf;
        {
            let mut containers = self.containers.write().await;
            let metadata = containers
                .get_mut(id)
                .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))?;

            if metadata.info.state != ContainerState::Created {
                return Err(ShimError::InvalidState {
                    expected: "created".to_string(),
                    actual: metadata.info.state.to_string(),
                });
            }

            bundle_path = PathBuf::from(&metadata.info.bundle_path);

            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;

            metadata.info.state = ContainerState::Running;
            metadata.info.started_at = Some(now);
            self.save_container(metadata).await?;
        }

        // Use runc run with --detach to start the container in background
        // Redirect stdout/stderr to log files
        let runc_root = self.data_dir.join("runc");
        let pid_file = bundle_path.join("container.pid");
        let stdout_path = bundle_path.join("stdout.log");
        let stderr_path = bundle_path.join("stderr.log");

        let stdout_file = std::fs::File::create(&stdout_path)
            .map_err(|e| ShimError::Runc(format!("Failed to create stdout log: {}", e)))?;
        let stderr_file = std::fs::File::create(&stderr_path)
            .map_err(|e| ShimError::Runc(format!("Failed to create stderr log: {}", e)))?;

        tracing::info!(container_id = %id, bundle = ?bundle_path, "Starting container with runc run");

        let mut child = tokio::process::Command::new("runc")
            .arg("--root")
            .arg(&runc_root)
            .arg("run")
            .arg("--bundle")
            .arg(&bundle_path)
            .arg("--pid-file")
            .arg(&pid_file)
            .arg("--no-pivot")
            .arg("--detach")
            .arg(id)
            .stdin(std::process::Stdio::null())
            .stdout(stdout_file)
            .stderr(stderr_file)
            .spawn()
            .map_err(|e| ShimError::Runc(format!("Failed to spawn runc: {}", e)))?;

        let status = child.wait().await
            .map_err(|e| ShimError::Runc(format!("Failed to wait for runc: {}", e)))?;

        if !status.success() {
            tracing::error!(container_id = %id, status = ?status, "runc run failed");
            return Err(ShimError::Runc(format!("runc run failed with status: {}", status)));
        }

        // Read PID from pid file
        if let Ok(pid_str) = fs::read_to_string(&pid_file).await {
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                let mut containers = self.containers.write().await;
                if let Some(metadata) = containers.get_mut(id) {
                    metadata.info.pid = Some(pid);
                    let _ = self.save_container(metadata).await;
                }
            }
        }

        tracing::info!(container_id = %id, "Container started");
        Ok(())
    }

    pub async fn stop(&self, id: &str, timeout: u32) -> Result<(), ShimError> {
        let mut containers = self.containers.write().await;
        let metadata = containers
            .get_mut(id)
            .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))?;

        if metadata.info.state != ContainerState::Running {
            return Err(ShimError::ContainerNotRunning(id.to_string()));
        }

        self.runc.kill(id, 15, None).await?;

        tokio::time::sleep(tokio::time::Duration::from_secs(timeout as u64)).await;

        let kill_opts = KillOpts::new().all(true);
        let _ = self.runc.kill(id, 9, Some(&kill_opts)).await;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        metadata.info.state = ContainerState::Stopped;
        metadata.info.finished_at = Some(now);
        metadata.info.pid = None;

        self.save_container(metadata).await?;

        tracing::info!(container_id = %id, "Container stopped");
        Ok(())
    }

    pub async fn kill(&self, id: &str, signal: u32) -> Result<(), ShimError> {
        let containers = self.containers.read().await;
        let metadata = containers
            .get(id)
            .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))?;

        if metadata.info.state != ContainerState::Running {
            return Err(ShimError::ContainerNotRunning(id.to_string()));
        }

        self.runc.kill(id, signal, None).await?;

        tracing::info!(container_id = %id, signal = signal, "Signal sent to container");
        Ok(())
    }

    pub async fn delete(&self, id: &str, force: bool) -> Result<(), ShimError> {
        let rootfs_path: PathBuf;
        {
            let containers = self.containers.read().await;
            let metadata = containers
                .get(id)
                .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))?;

            if metadata.info.state == ContainerState::Running && !force {
                return Err(ShimError::InvalidState {
                    expected: "stopped or created".to_string(),
                    actual: "running".to_string(),
                });
            }
            
            rootfs_path = PathBuf::from(&metadata.info.rootfs_path);
        }

        // Try to delete from runc, but ignore "container does not exist" errors
        // This can happen when a container exits and runc auto-cleans it
        let delete_opts = DeleteOpts::new().force(force);
        if let Err(e) = self.runc.delete(id, Some(&delete_opts)).await {
            let err_str = e.to_string();
            if !err_str.contains("does not exist") {
                return Err(e.into());
            }
            tracing::debug!(container_id = %id, "Container already removed from runc");
        }

        // Unmount the rootfs
        if rootfs_path.exists() {
            if let Err(e) = ross_mount::unmount(&rootfs_path) {
                tracing::warn!("Failed to unmount rootfs: {}", e);
            }
        }

        let container_dir = self.data_dir.join("containers").join(id);
        if container_dir.exists() {
            fs::remove_dir_all(&container_dir).await?;
        }

        {
            let mut containers = self.containers.write().await;
            containers.remove(id);
        }

        tracing::info!(container_id = %id, "Container deleted");
        Ok(())
    }

    pub async fn pause(&self, id: &str) -> Result<(), ShimError> {
        let mut containers = self.containers.write().await;
        let metadata = containers
            .get_mut(id)
            .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))?;

        if metadata.info.state != ContainerState::Running {
            return Err(ShimError::ContainerNotRunning(id.to_string()));
        }

        self.runc.pause(id).await?;
        metadata.info.state = ContainerState::Paused;
        self.save_container(metadata).await?;

        tracing::info!(container_id = %id, "Container paused");
        Ok(())
    }

    pub async fn resume(&self, id: &str) -> Result<(), ShimError> {
        let mut containers = self.containers.write().await;
        let metadata = containers
            .get_mut(id)
            .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))?;

        if metadata.info.state != ContainerState::Paused {
            return Err(ShimError::InvalidState {
                expected: "paused".to_string(),
                actual: metadata.info.state.to_string(),
            });
        }

        self.runc.resume(id).await?;
        metadata.info.state = ContainerState::Running;
        self.save_container(metadata).await?;

        tracing::info!(container_id = %id, "Container resumed");
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<ContainerInfo>, ShimError> {
        let containers = self.containers.read().await;
        Ok(containers.values().map(|m| m.info.clone()).collect())
    }

    pub async fn get(&self, id: &str) -> Result<ContainerInfo, ShimError> {
        let containers = self.containers.read().await;
        containers
            .get(id)
            .map(|m| m.info.clone())
            .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))
    }

    pub async fn wait(&self, id: &str) -> Result<WaitResult, ShimError> {
        let runc_root = self.data_dir.join("runc");
        
        loop {
            // Check runc state to see if container is still running
            let output = tokio::process::Command::new("runc")
                .arg("--root")
                .arg(&runc_root)
                .arg("state")
                .arg(id)
                .output()
                .await
                .map_err(|e| ShimError::Runc(format!("Failed to get runc state: {}", e)))?;
            
            let container_gone = !output.status.success();
            let is_stopped = if !container_gone {
                let state_json: serde_json::Value = serde_json::from_slice(&output.stdout)
                    .map_err(|e| ShimError::Runc(format!("Failed to parse runc state: {}", e)))?;
                let status = state_json["status"].as_str().unwrap_or("");
                tracing::debug!(container_id = %id, status = %status, "Container status");
                status == "stopped"
            } else {
                true
            };
            
            if container_gone || is_stopped {
                tracing::info!(container_id = %id, "Container has stopped");
                
                // Update internal state
                let mut containers = self.containers.write().await;
                if let Some(metadata) = containers.get_mut(id) {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64;
                    metadata.info.state = ContainerState::Stopped;
                    metadata.info.finished_at = Some(now);
                    metadata.info.exit_code = Some(0); // TODO: get actual exit code
                    let _ = self.save_container(metadata).await;
                }
                
                return Ok(WaitResult {
                    exit_code: 0,
                    error: None,
                });
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }

    /// Run a container and stream its output. This is a combined start+wait operation
    /// that captures stdout/stderr in real-time.
    pub fn run_streaming(
        &self,
        id: String,
    ) -> impl futures::Stream<Item = Result<OutputEvent, ShimError>> + Send + 'static {
        let data_dir = self.data_dir.clone();
        let containers = self.containers.clone();
        
        async_stream::try_stream! {
            let bundle_path: PathBuf;
            {
                let mut containers_guard = containers.write().await;
                let metadata = containers_guard
                    .get_mut(&id)
                    .ok_or_else(|| ShimError::ContainerNotFound(id.clone()))?;

                if metadata.info.state != ContainerState::Created {
                    Err(ShimError::InvalidState {
                        expected: "created".to_string(),
                        actual: metadata.info.state.to_string(),
                    })?;
                }

                bundle_path = PathBuf::from(&metadata.info.bundle_path);

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;

                metadata.info.state = ContainerState::Running;
                metadata.info.started_at = Some(now);
                
                let container_dir = data_dir.join("containers").join(&metadata.info.id);
                fs::create_dir_all(&container_dir).await?;
                let metadata_path = container_dir.join("metadata.json");
                let content = serde_json::to_string_pretty(&metadata)?;
                fs::write(&metadata_path, content).await?;
            }

            let runc_root = data_dir.join("runc");
            let pid_file = bundle_path.join("container.pid");

            tracing::info!(container_id = %id, bundle = ?bundle_path, "Starting container with runc run (streaming)");

            let mut child = tokio::process::Command::new("runc")
                .arg("--root")
                .arg(&runc_root)
                .arg("run")
                .arg("--bundle")
                .arg(&bundle_path)
                .arg("--pid-file")
                .arg(&pid_file)
                .arg("--no-pivot")
                .arg(&id)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| ShimError::Runc(format!("Failed to spawn runc: {}", e)))?;

            let stdout = child.stdout.take()
                .ok_or_else(|| ShimError::Runc("Failed to capture stdout".to_string()))?;
            let stderr = child.stderr.take()
                .ok_or_else(|| ShimError::Runc("Failed to capture stderr".to_string()))?;

            let mut stdout_reader = tokio::io::BufReader::new(stdout);
            let mut stderr_reader = tokio::io::BufReader::new(stderr);
            
            let mut stdout_buf = vec![0u8; 4096];
            let mut stderr_buf = vec![0u8; 4096];

            loop {
                tokio::select! {
                    result = tokio::io::AsyncReadExt::read(&mut stdout_reader, &mut stdout_buf) => {
                        match result {
                            Ok(0) => {}, // EOF on stdout
                            Ok(n) => {
                                yield OutputEvent::Stdout(stdout_buf[..n].to_vec());
                            }
                            Err(e) => {
                                tracing::warn!("Error reading stdout: {}", e);
                            }
                        }
                    }
                    result = tokio::io::AsyncReadExt::read(&mut stderr_reader, &mut stderr_buf) => {
                        match result {
                            Ok(0) => {}, // EOF on stderr
                            Ok(n) => {
                                yield OutputEvent::Stderr(stderr_buf[..n].to_vec());
                            }
                            Err(e) => {
                                tracing::warn!("Error reading stderr: {}", e);
                            }
                        }
                    }
                    status = child.wait() => {
                        let exit_code = match status {
                            Ok(s) => s.code().unwrap_or(-1),
                            Err(e) => {
                                tracing::error!("Error waiting for child: {}", e);
                                -1
                            }
                        };

                        // Update internal state
                        let mut containers_guard = containers.write().await;
                        if let Some(metadata) = containers_guard.get_mut(&id) {
                            let now = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
                                .as_secs() as i64;
                            metadata.info.state = ContainerState::Stopped;
                            metadata.info.finished_at = Some(now);
                            metadata.info.exit_code = Some(exit_code);
                            
                            let container_dir = data_dir.join("containers").join(&metadata.info.id);
                            let metadata_path = container_dir.join("metadata.json");
                            if let Ok(content) = serde_json::to_string_pretty(&metadata) {
                                let _ = fs::write(&metadata_path, content).await;
                            }
                        }

                        tracing::info!(container_id = %id, exit_code = exit_code, "Container exited");
                        
                        yield OutputEvent::Exit(WaitResult {
                            exit_code,
                            error: None,
                        });
                        
                        break;
                    }
                }
            }
        }
    }

    fn generate_spec(&self, opts: &CreateContainerOpts, rootfs: &Path) -> Result<Spec, ShimError> {
        let args = if !opts.config.entrypoint.is_empty() {
            let mut args = opts.config.entrypoint.clone();
            args.extend(opts.config.cmd.clone());
            args
        } else if !opts.config.cmd.is_empty() {
            opts.config.cmd.clone()
        } else {
            vec!["/bin/sh".to_string()]
        };

        let cwd = opts
            .config
            .working_dir
            .clone()
            .unwrap_or_else(|| "/".to_string());

        let env: Vec<String> = if opts.config.env.is_empty() {
            vec!["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string()]
        } else {
            opts.config.env.clone()
        };

        let user = opts.config.user.clone().unwrap_or_default();
        let (uid, gid) = parse_user(&user);

        let process = ProcessBuilder::default()
            .terminal(false)  // Disable TTY - requires console socket support
            .user(
                oci_spec::runtime::UserBuilder::default()
                    .uid(uid)
                    .gid(gid)
                    .build()
                    .map_err(|e| ShimError::OciSpec(e.to_string()))?,
            )
            .args(args)
            .env(env)
            .cwd(cwd)
            .no_new_privileges(true)
            .build()
            .map_err(|e| ShimError::OciSpec(e.to_string()))?;

        let root = RootBuilder::default()
            .path(rootfs)
            .readonly(opts.host_config.readonly_rootfs)
            .build()
            .map_err(|e| ShimError::OciSpec(e.to_string()))?;

        let mounts = self.generate_mounts(&opts.host_config)?;

        let namespaces = self.generate_namespaces(&opts.host_config)?;

        let linux = LinuxBuilder::default()
            .namespaces(namespaces)
            .build()
            .map_err(|e| ShimError::OciSpec(e.to_string()))?;

        let hostname = opts
            .config
            .hostname
            .clone()
            .unwrap_or_else(|| "container".to_string());

        let spec = SpecBuilder::default()
            .version("1.0.2")
            .root(root)
            .process(process)
            .hostname(hostname)
            .mounts(mounts)
            .linux(linux)
            .build()
            .map_err(|e| ShimError::OciSpec(e.to_string()))?;

        Ok(spec)
    }

    fn generate_mounts(&self, host_config: &HostConfig) -> Result<Vec<Mount>, ShimError> {
        let mut mounts = vec![
            MountBuilder::default()
                .destination("/proc")
                .typ("proc")
                .source("proc")
                .build()
                .map_err(|e| ShimError::OciSpec(e.to_string()))?,
            MountBuilder::default()
                .destination("/dev")
                .typ("tmpfs")
                .source("tmpfs")
                .options(vec![
                    "nosuid".to_string(),
                    "strictatime".to_string(),
                    "mode=755".to_string(),
                    "size=65536k".to_string(),
                ])
                .build()
                .map_err(|e| ShimError::OciSpec(e.to_string()))?,
            MountBuilder::default()
                .destination("/dev/pts")
                .typ("devpts")
                .source("devpts")
                .options(vec![
                    "nosuid".to_string(),
                    "noexec".to_string(),
                    "newinstance".to_string(),
                    "ptmxmode=0666".to_string(),
                    "mode=0620".to_string(),
                ])
                .build()
                .map_err(|e| ShimError::OciSpec(e.to_string()))?,
            MountBuilder::default()
                .destination("/dev/shm")
                .typ("tmpfs")
                .source("shm")
                .options(vec![
                    "nosuid".to_string(),
                    "noexec".to_string(),
                    "nodev".to_string(),
                    "mode=1777".to_string(),
                    "size=65536k".to_string(),
                ])
                .build()
                .map_err(|e| ShimError::OciSpec(e.to_string()))?,
            MountBuilder::default()
                .destination("/sys")
                .typ("sysfs")
                .source("sysfs")
                .options(vec![
                    "nosuid".to_string(),
                    "noexec".to_string(),
                    "nodev".to_string(),
                    "ro".to_string(),
                ])
                .build()
                .map_err(|e| ShimError::OciSpec(e.to_string()))?,
        ];

        for bind in &host_config.binds {
            let parts: Vec<&str> = bind.split(':').collect();
            if parts.len() >= 2 {
                let options = if parts.len() > 2 {
                    parts[2].split(',').map(|s| s.to_string()).collect()
                } else {
                    vec!["rbind".to_string(), "rprivate".to_string()]
                };

                mounts.push(
                    MountBuilder::default()
                        .destination(parts[1])
                        .typ("bind")
                        .source(parts[0])
                        .options(options)
                        .build()
                        .map_err(|e| ShimError::OciSpec(e.to_string()))?,
                );
            }
        }

        Ok(mounts)
    }

    fn generate_namespaces(
        &self,
        host_config: &HostConfig,
    ) -> Result<Vec<LinuxNamespace>, ShimError> {
        let mut namespaces = vec![
            LinuxNamespaceBuilder::default()
                .typ(LinuxNamespaceType::Pid)
                .build()
                .map_err(|e| ShimError::OciSpec(e.to_string()))?,
            LinuxNamespaceBuilder::default()
                .typ(LinuxNamespaceType::Ipc)
                .build()
                .map_err(|e| ShimError::OciSpec(e.to_string()))?,
            LinuxNamespaceBuilder::default()
                .typ(LinuxNamespaceType::Uts)
                .build()
                .map_err(|e| ShimError::OciSpec(e.to_string()))?,
            LinuxNamespaceBuilder::default()
                .typ(LinuxNamespaceType::Mount)
                .build()
                .map_err(|e| ShimError::OciSpec(e.to_string()))?,
        ];

        let use_host_network = host_config
            .network_mode
            .as_ref()
            .map(|m| m == "host")
            .unwrap_or(false);

        if !use_host_network {
            namespaces.push(
                LinuxNamespaceBuilder::default()
                    .typ(LinuxNamespaceType::Network)
                    .build()
                    .map_err(|e| ShimError::OciSpec(e.to_string()))?,
            );
        }

        Ok(namespaces)
    }
}

fn parse_user(user: &str) -> (u32, u32) {
    if user.is_empty() {
        return (0, 0);
    }

    let parts: Vec<&str> = user.split(':').collect();
    let uid = parts
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let gid = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(uid);

    (uid, gid)
}
