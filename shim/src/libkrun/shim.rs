//! KrunShim implementation - main shim logic.

use super::container::ContainerMetadata;
use super::rootfs as krun_rootfs;
use crate::error::ShimError;
use crate::rootfs;
use crate::shim::{OutputEventStream, Shim};
use crate::types::*;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::sync::RwLock;
use uuid::Uuid;

pub struct KrunShim {
    data_dir: PathBuf,
    containers: Arc<RwLock<HashMap<String, ContainerMetadata>>>,
}

impl KrunShim {
    pub async fn new(data_dir: &Path) -> Result<Self, ShimError> {
        let containers_dir = data_dir.join("containers");
        fs::create_dir_all(&containers_dir).await?;

        let shim = Self {
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
            if metadata_path.exists()
                && let Ok(metadata) = ContainerMetadata::load(&metadata_path).await
            {
                containers.insert(metadata.info.id.clone(), metadata);
            }
        }

        Ok(())
    }

    async fn save_container(&self, metadata: &ContainerMetadata) -> Result<(), ShimError> {
        let container_dir = self.data_dir.join("containers").join(&metadata.info.id);
        metadata.save(&container_dir).await
    }

    fn current_timestamp() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    fn container_dir(&self, id: &str) -> PathBuf {
        self.data_dir.join("containers").join(id)
    }
}

#[async_trait]
impl Shim for KrunShim {
    async fn create(&self, opts: CreateContainerOpts) -> Result<String, ShimError> {
        let id = Uuid::new_v4().to_string();

        {
            let containers = self.containers.read().await;
            if containers.contains_key(&id) {
                return Err(ShimError::ContainerAlreadyExists(id));
            }
        }

        let bundle_path = self.container_dir(&id).join("bundle");
        let rootfs_path = bundle_path.join("rootfs");
        fs::create_dir_all(&bundle_path).await?;

        if !opts.mounts.is_empty() {
            tracing::info!(container_id = %id, "Preparing rootfs from {} mount(s)", opts.mounts.len());
            krun_rootfs::prepare_from_mounts(&opts.mounts, &rootfs_path).await?;
        } else {
            tracing::info!(container_id = %id, "No mounts provided, creating minimal rootfs");
            rootfs::create_minimal_rootfs(&rootfs_path).await?;
        }

        let now = Self::current_timestamp();

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

        tracing::info!(container_id = %id, "Container created (libkrun)");
        Ok(id)
    }

    async fn start(&self, id: &str) -> Result<(), ShimError> {
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

        metadata.info.state = ContainerState::Running;
        metadata.info.started_at = Some(Self::current_timestamp());
        self.save_container(metadata).await?;

        tracing::info!(container_id = %id, "Container started (libkrun)");
        Ok(())
    }

    async fn stop(&self, id: &str, _timeout: u32) -> Result<(), ShimError> {
        let mut containers = self.containers.write().await;
        let metadata = containers
            .get_mut(id)
            .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))?;

        if metadata.info.state != ContainerState::Running {
            return Err(ShimError::ContainerNotRunning(id.to_string()));
        }

        metadata.info.state = ContainerState::Stopped;
        metadata.info.finished_at = Some(Self::current_timestamp());
        metadata.info.pid = None;
        self.save_container(metadata).await?;

        tracing::info!(container_id = %id, "Container stopped (libkrun)");
        Ok(())
    }

    async fn kill(&self, id: &str, signal: u32) -> Result<(), ShimError> {
        let containers = self.containers.read().await;
        let metadata = containers
            .get(id)
            .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))?;

        if metadata.info.state != ContainerState::Running {
            return Err(ShimError::ContainerNotRunning(id.to_string()));
        }

        tracing::info!(container_id = %id, signal = signal, "Signal sent to container (libkrun)");
        Ok(())
    }

    async fn delete(&self, id: &str, force: bool) -> Result<(), ShimError> {
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
        }

        let container_dir = self.container_dir(id);
        if container_dir.exists() {
            fs::remove_dir_all(&container_dir).await?;
        }

        {
            let mut containers = self.containers.write().await;
            containers.remove(id);
        }

        tracing::info!(container_id = %id, "Container deleted (libkrun)");
        Ok(())
    }

    async fn pause(&self, id: &str) -> Result<(), ShimError> {
        let mut containers = self.containers.write().await;
        let metadata = containers
            .get_mut(id)
            .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))?;

        if metadata.info.state != ContainerState::Running {
            return Err(ShimError::ContainerNotRunning(id.to_string()));
        }

        metadata.info.state = ContainerState::Paused;
        self.save_container(metadata).await?;

        tracing::info!(container_id = %id, "Container paused (libkrun)");
        Ok(())
    }

    async fn resume(&self, id: &str) -> Result<(), ShimError> {
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

        metadata.info.state = ContainerState::Running;
        self.save_container(metadata).await?;

        tracing::info!(container_id = %id, "Container resumed (libkrun)");
        Ok(())
    }

    async fn list(&self) -> Result<Vec<ContainerInfo>, ShimError> {
        let containers = self.containers.read().await;
        Ok(containers.values().map(|m| m.info.clone()).collect())
    }

    async fn get(&self, id: &str) -> Result<ContainerInfo, ShimError> {
        let containers = self.containers.read().await;
        containers
            .get(id)
            .map(|m| m.info.clone())
            .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))
    }

    async fn wait(&self, id: &str) -> Result<WaitResult, ShimError> {
        loop {
            {
                let containers = self.containers.read().await;
                if let Some(metadata) = containers.get(id) {
                    if metadata.info.state == ContainerState::Stopped {
                        return Ok(WaitResult {
                            exit_code: metadata.info.exit_code.unwrap_or(0),
                            error: None,
                        });
                    }
                } else {
                    return Err(ShimError::ContainerNotFound(id.to_string()));
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }

    #[allow(unused_variables)]
    fn run_streaming(&self, id: String) -> OutputEventStream {
        #[cfg(all(feature = "libkrun", target_os = "macos"))]
        {
            use super::krun;
            use std::os::unix::io::FromRawFd;

            let containers = self.containers.clone();
            let data_dir = self.data_dir.clone();

            Box::pin(async_stream::try_stream! {
                let (config, rootfs_path): (ContainerConfig, PathBuf);
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

                    config = metadata.config.clone();
                    rootfs_path = PathBuf::from(&metadata.info.rootfs_path);

                    metadata.info.state = ContainerState::Running;
                    metadata.info.started_at = Some(KrunShim::current_timestamp());
                    metadata.save(&data_dir.join("containers").join(&id)).await?;
                }

                tracing::info!(container_id = %id, rootfs = ?rootfs_path, "Starting container with libkrun (streaming)");

                krun::fix_root_mode(&rootfs_path);

                let (exec_path, argv) = if !config.entrypoint.is_empty() {
                    let mut args = config.entrypoint.clone();
                    args.extend(config.cmd.clone());
                    (config.entrypoint[0].clone(), args)
                } else if !config.cmd.is_empty() {
                    (config.cmd[0].clone(), config.cmd.clone())
                } else {
                    ("/bin/sh".to_string(), vec!["/bin/sh".to_string()])
                };

                let workdir = config.working_dir.as_deref();

                let (stdout_fd, child_pid) = krun::fork_and_run_vm(
                    &rootfs_path,
                    &exec_path,
                    &argv,
                    &config.env,
                    workdir,
                )?;

                let stdout_file = unsafe { std::fs::File::from_raw_fd(stdout_fd) };
                let (output_tx, mut output_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);

                std::thread::spawn(move || {
                    use std::io::Read;
                    let mut reader = std::io::BufReader::new(stdout_file);
                    let mut buf = [0u8; 4096];
                    loop {
                        match reader.get_mut().read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                if output_tx.blocking_send(buf[..n].to_vec()).is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                });

                let containers_for_wait = containers.clone();
                let id_for_wait = id.clone();
                let data_dir_for_wait = data_dir.clone();

                tokio::task::spawn_blocking(move || {
                    let exit_code = krun::wait_for_child(child_pid);

                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();

                    rt.block_on(async {
                        let mut containers_guard = containers_for_wait.write().await;
                        if let Some(metadata) = containers_guard.get_mut(&id_for_wait) {
                            metadata.info.state = ContainerState::Stopped;
                            metadata.info.exit_code = Some(exit_code);
                            metadata.info.finished_at = Some(KrunShim::current_timestamp());
                            let _ = metadata.save(&data_dir_for_wait.join("containers").join(&id_for_wait)).await;
                        }
                    });
                });

                while let Some(data) = output_rx.recv().await {
                    yield OutputEvent::Stdout(data);
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                let exit_code = {
                    let containers_guard = containers.read().await;
                    containers_guard
                        .get(&id)
                        .and_then(|m| m.info.exit_code)
                        .unwrap_or(0)
                };

                yield OutputEvent::Exit(WaitResult {
                    exit_code,
                    error: None,
                });
            })
        }

        #[cfg(not(all(feature = "libkrun", target_os = "macos")))]
        {
            Box::pin(async_stream::try_stream! {
                yield OutputEvent::Exit(WaitResult {
                    exit_code: 1,
                    error: Some("libkrun support not available".to_string()),
                });
            })
        }
    }

    #[allow(unused_variables)]
    async fn run_interactive(
        &self,
        id: String,
        mut input_rx: tokio::sync::mpsc::Receiver<InputEvent>,
        output_tx: tokio::sync::mpsc::Sender<OutputEvent>,
    ) -> Result<(), ShimError> {
        #[cfg(all(feature = "libkrun", target_os = "macos"))]
        {
            use super::krun::{self, NetworkConfig};
            use super::net::{DEFAULT_MAC, VmNetwork, network_available};
            use crate::guest_config::GuestConfig;
            use crate::tty_host;
            use std::os::unix::net::UnixListener;

            let (config, rootfs_path): (ContainerConfig, PathBuf);
            {
                let mut containers = self.containers.write().await;
                let metadata = containers
                    .get_mut(&id)
                    .ok_or_else(|| ShimError::ContainerNotFound(id.clone()))?;

                if metadata.info.state != ContainerState::Created {
                    return Err(ShimError::InvalidState {
                        expected: "created".to_string(),
                        actual: metadata.info.state.to_string(),
                    });
                }

                config = metadata.config.clone();
                rootfs_path = PathBuf::from(&metadata.info.rootfs_path);

                metadata.info.state = ContainerState::Running;
                metadata.info.started_at = Some(Self::current_timestamp());
                self.save_container(metadata).await?;
            }

            tracing::info!(container_id = %id, rootfs = ?rootfs_path, tty = config.tty, "Starting container with libkrun (interactive)");

            krun::fix_root_mode(&rootfs_path);

            let (command, args) = if !config.entrypoint.is_empty() {
                let mut args = config.entrypoint[1..].to_vec();
                args.extend(config.cmd.clone());
                (config.entrypoint[0].clone(), args)
            } else if !config.cmd.is_empty() {
                (config.cmd[0].clone(), config.cmd[1..].to_vec())
            } else {
                ("/bin/sh".to_string(), vec![])
            };

            // Allocate a vsock port for communication
            let vsock_port = 50000 + (std::process::id() % 1000);
            let socket_path = krun::get_vsock_socket_path(vsock_port);

            // Remove old socket if it exists
            let _ = std::fs::remove_file(&socket_path);

            // Create Unix socket listener before starting VM
            let listener = UnixListener::bind(&socket_path).map_err(|e| {
                ShimError::RuntimeError(format!("Failed to bind vsock socket: {}", e))
            })?;

            let guest_config = GuestConfig {
                command,
                args,
                env: config.env.clone(),
                workdir: config.working_dir.clone(),
                tty: config.tty,
                vsock_port,
            };

            // Start userspace network stack if available
            let network = if network_available() {
                match VmNetwork::start(&id) {
                    Ok(n) => {
                        tracing::info!(container_id = %id, "Userspace network stack enabled");
                        Some(n)
                    }
                    Err(e) => {
                        tracing::warn!(container_id = %id, error = %e, "Failed to start network stack, falling back to TSI");
                        None
                    }
                }
            } else {
                tracing::debug!(container_id = %id, "Network stack not available, using TSI networking");
                None
            };

            // Prepare network config if network stack is running
            let network_config = network.as_ref().map(|n| NetworkConfig {
                socket_path: n.socket_path().to_string(),
                mac: DEFAULT_MAC,
            });

            // Fork and start VM
            let child_pid = krun::fork_and_run_vm_interactive_with_network(
                &rootfs_path,
                &guest_config,
                vsock_port,
                network_config,
            )?;

            let is_tty = config.tty;
            let containers = self.containers.clone();
            let data_dir = self.data_dir.clone();
            let id_clone = id.clone();

            // Create std::sync channels for the blocking I/O loop
            let (sync_input_tx, sync_input_rx) = std::sync::mpsc::channel::<InputEvent>();
            let (sync_output_tx, sync_output_rx) = std::sync::mpsc::channel::<OutputEvent>();

            // Spawn task to forward from tokio channel to std channel
            let input_forwarder = tokio::spawn(async move {
                while let Some(event) = input_rx.recv().await {
                    if sync_input_tx.send(event).is_err() {
                        break;
                    }
                }
            });

            // Spawn task to forward from std channel to tokio channel
            let output_tx_clone = output_tx.clone();
            let output_forwarder = tokio::task::spawn_blocking(move || {
                while let Ok(event) = sync_output_rx.recv() {
                    // We need to send to the tokio channel from a blocking context
                    // Use a runtime handle to do this
                    let tx = output_tx_clone.clone();
                    let _ = futures::executor::block_on(tx.send(event));
                }
            });

            // Run I/O loop in blocking task
            let io_result = tokio::task::spawn_blocking(move || {
                tty_host::run_io_host_with_channels(listener, is_tty, sync_input_rx, sync_output_tx)
            })
            .await
            .map_err(|e| ShimError::RuntimeError(format!("I/O task panicked: {}", e)))?;

            // Wait for child process
            let exit_code = tokio::task::spawn_blocking(move || krun::wait_for_child(child_pid))
                .await
                .unwrap_or(1);

            // Clean up socket
            let _ = std::fs::remove_file(&socket_path);

            // Cancel forwarders
            input_forwarder.abort();
            output_forwarder.abort();

            // Update container state
            {
                let mut containers_guard = containers.write().await;
                if let Some(metadata) = containers_guard.get_mut(&id_clone) {
                    metadata.info.state = ContainerState::Stopped;
                    metadata.info.exit_code = Some(exit_code);
                    metadata.info.finished_at = Some(Self::current_timestamp());
                    let _ = metadata
                        .save(&data_dir.join("containers").join(&id_clone))
                        .await;
                }
            }

            let final_exit_code = io_result.unwrap_or(exit_code as u8);
            let _ = output_tx
                .send(OutputEvent::Exit(WaitResult {
                    exit_code: final_exit_code as i32,
                    error: None,
                }))
                .await;

            Ok(())
        }

        #[cfg(not(all(feature = "libkrun", target_os = "macos")))]
        {
            let _ = input_rx;
            let _ = output_tx
                .send(OutputEvent::Exit(WaitResult {
                    exit_code: 1,
                    error: Some("libkrun support not available".to_string()),
                }))
                .await;
            Ok(())
        }
    }
}
