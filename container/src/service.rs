use crate::error::ContainerError;
use crate::types::*;
use async_stream::stream;
use ross_shim::{CreateContainerOpts, RuncShim};
use ross_snapshotter::OverlaySnapshotter;
use ross_store::FileSystemStore;
use std::collections::HashMap;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;

type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;

struct ImageConfigInfo {
    top_layer: Option<String>,
    entrypoint: Vec<String>,
    cmd: Vec<String>,
    env: Vec<String>,
    working_dir: String,
    user: String,
}

pub struct ContainerService {
    shim: Arc<RuncShim>,
    snapshotter: Arc<OverlaySnapshotter>,
    #[allow(dead_code)]
    store: Arc<FileSystemStore>,
}

impl ContainerService {
    pub async fn new(
        data_dir: &Path,
        snapshotter: Arc<OverlaySnapshotter>,
        store: Arc<FileSystemStore>,
    ) -> Result<Self, ContainerError> {
        let shim = RuncShim::new(&data_dir.join("shim")).await?;

        Ok(Self {
            shim: Arc::new(shim),
            snapshotter,
            store,
        })
    }

    pub async fn create(
        &self,
        params: CreateContainerParams,
    ) -> Result<CreateContainerResult, ContainerError> {
        tracing::info!("Creating container with name: {:?}", params.name);

        let image_ref = &params.config.image;
        tracing::info!("Looking up image: {}", image_ref);

        // Get image config (includes top layer and default entrypoint/cmd)
        let image_config = self.get_image_config(image_ref).await?;
        
        let top_layer_digest = image_config.top_layer.ok_or_else(|| {
            ContainerError::ImageNotFound("Image has no layers".to_string())
        })?;
        tracing::info!("Found top layer: {}", top_layer_digest);

        // Verify the layer snapshot exists
        if self.snapshotter.stat(&top_layer_digest).await.is_err() {
            return Err(ContainerError::ImageNotFound(format!(
                "Layer snapshot not found: {}. Did you pull the image first?",
                top_layer_digest
            )));
        }

        let snapshot_key = format!("container-{}", uuid::Uuid::new_v4());

        let mut labels = HashMap::new();
        labels.insert("container".to_string(), "true".to_string());
        labels.insert("image".to_string(), image_ref.clone());

        tracing::info!(
            "Creating container snapshot {} from layer {}",
            snapshot_key,
            top_layer_digest
        );

        let mounts = self
            .snapshotter
            .prepare(&snapshot_key, Some(&top_layer_digest), labels)
            .await?;

        // Convert snapshotter mounts to shim mounts
        let shim_mounts: Vec<ross_shim::SnapshotMount> = mounts
            .iter()
            .map(|m| ross_shim::SnapshotMount {
                mount_type: m.mount_type.clone(),
                source: m.source.clone(),
                options: m.options.clone(),
            })
            .collect();

        tracing::info!("Prepared {} mount(s) for container", shim_mounts.len());

        // Merge user config with image config (user config takes precedence)
        let entrypoint = if params.config.entrypoint.is_empty() {
            image_config.entrypoint
        } else {
            params.config.entrypoint.clone()
        };

        let cmd = if params.config.cmd.is_empty() {
            image_config.cmd
        } else {
            params.config.cmd.clone()
        };

        let env = if params.config.env.is_empty() {
            image_config.env
        } else {
            // Merge: image env + user env (user overrides)
            let mut merged = image_config.env;
            merged.extend(params.config.env.clone());
            merged
        };

        let working_dir = if params.config.working_dir.is_empty() {
            if image_config.working_dir.is_empty() {
                None
            } else {
                Some(image_config.working_dir)
            }
        } else {
            Some(params.config.working_dir.clone())
        };

        let user = if params.config.user.is_empty() {
            if image_config.user.is_empty() {
                None
            } else {
                Some(image_config.user)
            }
        } else {
            Some(params.config.user.clone())
        };

        tracing::info!("Container entrypoint: {:?}, cmd: {:?}", entrypoint, cmd);

        let shim_config = ross_shim::ContainerConfig {
            image: params.config.image.clone(),
            hostname: if params.config.hostname.is_empty() {
                None
            } else {
                Some(params.config.hostname.clone())
            },
            user,
            env,
            cmd,
            entrypoint,
            working_dir,
            labels: params.config.labels.clone(),
            tty: params.config.tty,
            open_stdin: params.config.open_stdin,
        };

        let shim_host_config = ross_shim::HostConfig {
            binds: params.host_config.binds.clone(),
            network_mode: if params.host_config.network_mode.is_empty() {
                None
            } else {
                Some(params.host_config.network_mode.clone())
            },
            privileged: params.host_config.privileged,
            readonly_rootfs: params.host_config.readonly_rootfs,
            auto_remove: params.host_config.auto_remove,
        };

        let opts = CreateContainerOpts {
            name: params.name.clone(),
            config: shim_config,
            host_config: shim_host_config,
            mounts: shim_mounts,
        };

        let id = self.shim.create(opts).await?;

        Ok(CreateContainerResult {
            id,
            warnings: vec![],
        })
    }

    async fn get_image_top_layer(&self, image_ref: &str) -> Result<String, ContainerError> {
        let image_config = self.get_image_config(image_ref).await?;
        
        image_config.top_layer.ok_or_else(|| {
            ContainerError::ImageNotFound("Image has no layers".to_string())
        })
    }

    async fn get_image_config(&self, image_ref: &str) -> Result<ImageConfigInfo, ContainerError> {
        let (repository, tag) = parse_image_reference(image_ref);
        
        tracing::debug!("Looking up image {}:{}", repository, tag);

        let tags = self.store.list_tags(&repository).await.map_err(|e| {
            ContainerError::ImageNotFound(format!("Failed to list tags for {}: {}", repository, e))
        })?;

        let tag_info = tags.iter().find(|t| t.tag == tag).ok_or_else(|| {
            ContainerError::ImageNotFound(format!("Tag {} not found for repository {}", tag, repository))
        })?;

        let manifest_digest = tag_info.digest.as_ref().ok_or_else(|| {
            ContainerError::ImageNotFound(format!("No digest for tag {}:{}", repository, tag))
        })?;

        let (manifest_bytes, _media_type) = self.store.get_manifest(manifest_digest).await.map_err(|e| {
            ContainerError::ImageNotFound(format!("Failed to get manifest: {}", e))
        })?;

        #[derive(serde::Deserialize)]
        struct Manifest {
            config: ConfigDescriptor,
            layers: Vec<LayerDescriptor>,
        }
        #[derive(serde::Deserialize)]
        struct ConfigDescriptor {
            digest: String,
        }
        #[derive(serde::Deserialize)]
        struct LayerDescriptor {
            digest: String,
        }

        let manifest: Manifest = serde_json::from_slice(&manifest_bytes).map_err(|e| {
            ContainerError::ImageNotFound(format!("Failed to parse manifest: {}", e))
        })?;

        let top_layer = manifest.layers.last().map(|l| l.digest.clone());

        // Get the image config blob
        let config_digest = ross_store::Digest {
            algorithm: "sha256".to_string(),
            hash: manifest.config.digest.trim_start_matches("sha256:").to_string(),
        };

        let config_bytes = self.store.get_blob(&config_digest, 0, -1).await.map_err(|e| {
            ContainerError::ImageNotFound(format!("Failed to get image config: {}", e))
        })?;

        #[derive(serde::Deserialize)]
        struct ImageConfig {
            config: Option<ContainerConfigBlob>,
        }
        #[derive(serde::Deserialize)]
        struct ContainerConfigBlob {
            #[serde(rename = "Entrypoint")]
            entrypoint: Option<Vec<String>>,
            #[serde(rename = "Cmd")]
            cmd: Option<Vec<String>>,
            #[serde(rename = "Env")]
            env: Option<Vec<String>>,
            #[serde(rename = "WorkingDir")]
            working_dir: Option<String>,
            #[serde(rename = "User")]
            user: Option<String>,
        }

        let image_config: ImageConfig = serde_json::from_slice(&config_bytes).map_err(|e| {
            ContainerError::ImageNotFound(format!("Failed to parse image config: {}", e))
        })?;

        let container_config = image_config.config.unwrap_or(ContainerConfigBlob {
            entrypoint: None,
            cmd: None,
            env: None,
            working_dir: None,
            user: None,
        });

        Ok(ImageConfigInfo {
            top_layer,
            entrypoint: container_config.entrypoint.unwrap_or_default(),
            cmd: container_config.cmd.unwrap_or_default(),
            env: container_config.env.unwrap_or_default(),
            working_dir: container_config.working_dir.unwrap_or_default(),
            user: container_config.user.unwrap_or_default(),
        })
    }

    pub async fn start(&self, container_id: &str) -> Result<(), ContainerError> {
        tracing::info!("Starting container: {}", container_id);
        self.shim.start(container_id).await?;
        Ok(())
    }

    pub async fn stop(&self, container_id: &str, timeout: i32) -> Result<(), ContainerError> {
        tracing::info!(
            "Stopping container: {} with timeout: {}",
            container_id,
            timeout
        );
        self.shim.stop(container_id, timeout as u32).await?;
        Ok(())
    }

    pub async fn restart(&self, container_id: &str, timeout: i32) -> Result<(), ContainerError> {
        tracing::info!(
            "Restarting container: {} with timeout: {}",
            container_id,
            timeout
        );
        self.shim.stop(container_id, timeout as u32).await?;
        self.shim.start(container_id).await?;
        Ok(())
    }

    pub async fn list(
        &self,
        params: ListContainersParams,
    ) -> Result<Vec<Container>, ContainerError> {
        tracing::info!(
            "Listing containers (all: {}, limit: {})",
            params.all,
            params.limit
        );

        let containers = self.shim.list().await?;

        let mut result: Vec<Container> = containers
            .into_iter()
            .filter(|c| params.all || c.state == ross_shim::ContainerState::Running)
            .map(|c| Container {
                id: c.id.clone(),
                names: c.name.map(|n| vec![n]).unwrap_or_default(),
                image: c.image.clone(),
                image_id: String::new(),
                command: String::new(),
                created: Some(prost_types::Timestamp {
                    seconds: c.created_at,
                    nanos: 0,
                }),
                state: c.state.to_string(),
                status: c.state.to_string(),
                ports: vec![],
                labels: std::collections::HashMap::new(),
                size_rw: 0,
                size_root_fs: 0,
            })
            .collect();

        if params.limit > 0 {
            result.truncate(params.limit as usize);
        }

        Ok(result)
    }

    pub async fn inspect(&self, container_id: &str) -> Result<ContainerInspection, ContainerError> {
        tracing::info!("Inspecting container: {}", container_id);

        let info = self.shim.get(container_id).await?;

        let state = ContainerState {
            status: info.state.to_string(),
            running: info.state == ross_shim::ContainerState::Running,
            paused: info.state == ross_shim::ContainerState::Paused,
            restarting: false,
            oom_killed: false,
            dead: false,
            pid: info.pid.map(|p| p as i32).unwrap_or(0),
            exit_code: info.exit_code.unwrap_or(0),
            error: String::new(),
            started_at: info.started_at.map(|t| prost_types::Timestamp {
                seconds: t,
                nanos: 0,
            }),
            finished_at: info.finished_at.map(|t| prost_types::Timestamp {
                seconds: t,
                nanos: 0,
            }),
        };

        let container = Container {
            id: info.id.clone(),
            names: info.name.clone().map(|n| vec![n]).unwrap_or_default(),
            image: info.image.clone(),
            image_id: String::new(),
            command: String::new(),
            created: Some(prost_types::Timestamp {
                seconds: info.created_at,
                nanos: 0,
            }),
            state: info.state.to_string(),
            status: info.state.to_string(),
            ports: vec![],
            labels: std::collections::HashMap::new(),
            size_rw: 0,
            size_root_fs: 0,
        };

        Ok(ContainerInspection {
            container,
            state,
            path: String::new(),
            args: vec![],
            resolv_conf_path: String::new(),
            hostname_path: String::new(),
            hosts_path: String::new(),
            log_path: String::new(),
            name: info.name.unwrap_or_default(),
            restart_count: 0,
            driver: "overlay".to_string(),
            platform: "linux".to_string(),
            mount_label: String::new(),
            process_label: String::new(),
            app_armor_profile: String::new(),
            exec_ids: vec![],
            config: ContainerConfig::default(),
            host_config: HostConfig::default(),
        })
    }

    pub async fn remove(
        &self,
        container_id: &str,
        force: bool,
        _remove_volumes: bool,
    ) -> Result<(), ContainerError> {
        tracing::info!("Removing container: {} (force: {})", container_id, force);
        self.shim.delete(container_id, force).await?;
        Ok(())
    }

    pub async fn pause(&self, container_id: &str) -> Result<(), ContainerError> {
        tracing::info!("Pausing container: {}", container_id);
        self.shim.pause(container_id).await?;
        Ok(())
    }

    pub async fn unpause(&self, container_id: &str) -> Result<(), ContainerError> {
        tracing::info!("Unpausing container: {}", container_id);
        self.shim.resume(container_id).await?;
        Ok(())
    }

    pub fn get_logs(&self, params: GetLogsParams) -> BoxStream<Result<LogEntry, ContainerError>> {
        tracing::info!(
            "Getting logs for container: {} (follow: {})",
            params.container_id,
            params.follow
        );

        let output = stream! {
            let log_messages = [
                ("stdout", "Container started"),
                ("stdout", "Application running"),
                ("stderr", "Health check passed"),
            ];

            for (stream_type, message) in log_messages {
                yield Ok(LogEntry {
                    timestamp: now_timestamp(),
                    stream: stream_type.to_string(),
                    message: message.to_string(),
                });
            }
        };

        Box::pin(output)
    }

    pub async fn exec_create(
        &self,
        container_id: &str,
        config: ExecConfig,
    ) -> Result<String, ContainerError> {
        tracing::info!(
            "Creating exec instance in container: {} with cmd: {:?}",
            container_id,
            config.cmd
        );
        Ok("stub-exec-id".to_string())
    }

    pub fn exec_start(&self, exec_id: &str) -> BoxStream<Result<ExecOutput, ContainerError>> {
        tracing::info!("Starting exec: {}", exec_id);

        let output = stream! {
            let outputs = [
                "Command executed successfully\n",
                "Output line 1\n",
                "Output line 2\n",
            ];

            for data in outputs {
                yield Ok(ExecOutput {
                    stream: "stdout".to_string(),
                    data: data.as_bytes().to_vec(),
                });
            }
        };

        Box::pin(output)
    }

    pub fn attach<S>(&self, input_stream: S) -> BoxStream<Result<AttachOutput, ContainerError>>
    where
        S: Stream<Item = Result<AttachInput, ContainerError>> + Send + 'static,
    {
        use tokio_stream::StreamExt;
        tracing::info!("Attaching to container");

        let output = stream! {
            tokio::pin!(input_stream);
            while let Some(result) = input_stream.next().await {
                match result {
                    Ok(attach_input) => {
                        tracing::info!(
                            "Received attach input for container: {}, {} bytes",
                            attach_input.container_id,
                            attach_input.input.len()
                        );
                        yield Ok(AttachOutput {
                            stream: "stdout".to_string(),
                            data: attach_input.input,
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Error receiving attach input: {}", e);
                        break;
                    }
                }
            }
        };

        Box::pin(output)
    }

    pub fn wait_streaming(
        &self,
        container_id: &str,
    ) -> impl futures::Stream<Item = Result<OutputEvent, ContainerError>> + Send + 'static {
        use futures::StreamExt;
        
        tracing::info!("Waiting for container (streaming): {}", container_id);

        let stream = self.shim.run_streaming(container_id.to_string());
        
        stream.map(|result| {
            result
                .map(|event| match event {
                    ross_shim::OutputEvent::Stdout(data) => OutputEvent::Stdout(data),
                    ross_shim::OutputEvent::Stderr(data) => OutputEvent::Stderr(data),
                    ross_shim::OutputEvent::Exit(r) => OutputEvent::Exit(WaitResult {
                        status_code: r.exit_code as i64,
                        error: r.error,
                    }),
                })
                .map_err(ContainerError::from)
        })
    }

    pub async fn kill(&self, container_id: &str, signal: &str) -> Result<(), ContainerError> {
        tracing::info!(
            "Killing container: {} with signal: {}",
            container_id,
            signal
        );

        let sig = parse_signal(signal);
        self.shim.kill(container_id, sig).await?;

        Ok(())
    }

    pub async fn rename(&self, container_id: &str, new_name: &str) -> Result<(), ContainerError> {
        tracing::info!("Renaming container: {} to: {}", container_id, new_name);
        Ok(())
    }

    pub fn stats(&self, params: StatsParams) -> BoxStream<Result<ContainerStats, ContainerError>> {
        tracing::info!(
            "Getting stats for container: {} (stream: {})",
            params.container_id,
            params.stream
        );

        let output = stream! {
            for i in 0..3u64 {
                yield Ok(ContainerStats {
                    read: Some(now_timestamp()),
                    preread: Some(now_timestamp()),
                    num_procs: 1,
                    cpu_stats: Some(CpuStats {
                        cpu_usage: Some(CpuUsage {
                            total_usage: 1000000 * (i + 1),
                            percpu_usage: vec![500000 * (i + 1)],
                            usage_in_kernelmode: 100000 * (i + 1),
                            usage_in_usermode: 900000 * (i + 1),
                        }),
                        system_cpu_usage: 10000000000,
                        online_cpus: 4,
                    }),
                    precpu_stats: None,
                    memory_stats: Some(MemoryStats {
                        usage: 52428800 + (i * 1048576),
                        max_usage: 104857600,
                        stats: Default::default(),
                        failcnt: 0,
                        limit: 1073741824,
                        commit: 0,
                        commit_peak: 0,
                        private_working_set: 0,
                    }),
                    networks: Default::default(),
                });
            }
        };

        Box::pin(output)
    }

    /// Run a container interactively with bidirectional streaming.
    /// Returns a sender for input events and an output stream.
    pub async fn run_interactive(
        &self,
        container_id: String,
        tty: bool,
    ) -> Result<
        (
            tokio::sync::mpsc::Sender<InputEvent>,
            BoxStream<Result<OutputEvent, ContainerError>>,
        ),
        ContainerError,
    > {
        tracing::info!(
            "Starting interactive session for container: {} (tty: {})",
            container_id,
            tty
        );

        let (input_tx, input_rx) = tokio::sync::mpsc::channel::<InputEvent>(32);
        let (output_tx, mut output_rx) = tokio::sync::mpsc::channel::<ross_shim::OutputEvent>(32);

        // Convert InputEvent to shim::InputEvent
        let (shim_input_tx, shim_input_rx) = tokio::sync::mpsc::channel::<ross_shim::InputEvent>(32);

        let shim = self.shim.clone();
        let container_id_clone = container_id.clone();

        // Forward input events to shim format
        tokio::spawn(async move {
            let mut input_rx = input_rx;
            while let Some(event) = input_rx.recv().await {
                tracing::debug!("Forwarding input event to shim");
                let shim_event = match event {
                    InputEvent::Stdin(data) => {
                        tracing::debug!("Forwarding stdin: {} bytes", data.len());
                        ross_shim::InputEvent::Stdin(data)
                    }
                    InputEvent::Resize { width, height } => {
                        ross_shim::InputEvent::Resize { width, height }
                    }
                };
                if shim_input_tx.send(shim_event).await.is_err() {
                    tracing::debug!("Shim input channel closed");
                    break;
                }
            }
            tracing::debug!("Input forwarding task exiting");
        });

        // Start the interactive session in the shim
        tokio::spawn(async move {
            if let Err(e) = shim
                .run_interactive(container_id_clone, shim_input_rx, output_tx)
                .await
            {
                tracing::error!("Interactive session error: {}", e);
            }
        });

        // Create output stream from channel
        let output_stream = stream! {
            while let Some(event) = output_rx.recv().await {
                let result = match event {
                    ross_shim::OutputEvent::Stdout(data) => OutputEvent::Stdout(data),
                    ross_shim::OutputEvent::Stderr(data) => OutputEvent::Stderr(data),
                    ross_shim::OutputEvent::Exit(r) => OutputEvent::Exit(WaitResult {
                        status_code: r.exit_code as i64,
                        error: r.error,
                    }),
                };
                yield Ok(result);
            }
        };

        Ok((input_tx, Box::pin(output_stream)))
    }
}

fn parse_image_reference(image: &str) -> (String, String) {
    let image = image.trim();
    
    // Extract tag/digest
    let (name_part, tag) = if let Some(at_idx) = image.rfind('@') {
        (&image[..at_idx], &image[at_idx + 1..])
    } else if let Some(colon_idx) = image.rfind(':') {
        let potential_tag = &image[colon_idx + 1..];
        if !potential_tag.contains('/') {
            (&image[..colon_idx], potential_tag)
        } else {
            (image, "latest")
        }
    } else {
        (image, "latest")
    };
    
    // Determine repository - need to match how the store indexes images
    // The store uses the format from ImageReference which stores:
    // - "library/nginx" for "nginx"
    // - "myuser/myimage" for "myuser/myimage"
    let repository = if name_part.contains('/') {
        let first_slash = name_part.find('/').unwrap();
        let first_part = &name_part[..first_slash];
        
        // Check if first part is a registry
        if first_part.contains('.') || first_part.contains(':') || first_part == "localhost" {
            // Has registry - repository is everything after first /
            name_part[first_slash + 1..].to_string()
        } else {
            // No registry, whole thing is repository
            name_part.to_string()
        }
    } else {
        // Simple name like "nginx" -> "library/nginx"
        format!("library/{}", name_part)
    };
    
    (repository, tag.to_string())
}

fn parse_signal(signal: &str) -> u32 {
    match signal.to_uppercase().as_str() {
        "SIGKILL" | "KILL" | "9" => 9,
        "SIGTERM" | "TERM" | "15" => 15,
        "SIGINT" | "INT" | "2" => 2,
        "SIGHUP" | "HUP" | "1" => 1,
        "SIGQUIT" | "QUIT" | "3" => 3,
        "SIGUSR1" | "USR1" | "10" => 10,
        "SIGUSR2" | "USR2" | "12" => 12,
        _ => signal.parse().unwrap_or(15),
    }
}
