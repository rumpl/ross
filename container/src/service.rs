use crate::error::ContainerError;
use crate::types::*;
use async_stream::stream;
use std::pin::Pin;
use tokio_stream::Stream;

type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;

#[derive(Default)]
pub struct ContainerService;

impl ContainerService {
    pub fn new() -> Self {
        Self
    }

    pub async fn create(
        &self,
        params: CreateContainerParams,
    ) -> Result<CreateContainerResult, ContainerError> {
        tracing::info!("Creating container with name: {:?}", params.name);
        Ok(CreateContainerResult {
            id: "stub-container-id".to_string(),
            warnings: vec![],
        })
    }

    pub async fn start(&self, container_id: &str) -> Result<(), ContainerError> {
        tracing::info!("Starting container: {}", container_id);
        Ok(())
    }

    pub async fn stop(&self, container_id: &str, timeout: i32) -> Result<(), ContainerError> {
        tracing::info!(
            "Stopping container: {} with timeout: {}",
            container_id,
            timeout
        );
        Ok(())
    }

    pub async fn restart(&self, container_id: &str, timeout: i32) -> Result<(), ContainerError> {
        tracing::info!(
            "Restarting container: {} with timeout: {}",
            container_id,
            timeout
        );
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
        Ok(vec![])
    }

    pub async fn inspect(
        &self,
        container_id: &str,
    ) -> Result<ContainerInspection, ContainerError> {
        tracing::info!("Inspecting container: {}", container_id);
        Ok(ContainerInspection {
            container: Container::default(),
            state: ContainerState::default(),
            path: String::new(),
            args: vec![],
            resolv_conf_path: String::new(),
            hostname_path: String::new(),
            hosts_path: String::new(),
            log_path: String::new(),
            name: String::new(),
            restart_count: 0,
            driver: String::new(),
            platform: String::new(),
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
        remove_volumes: bool,
    ) -> Result<(), ContainerError> {
        tracing::info!(
            "Removing container: {} (force: {}, volumes: {})",
            container_id,
            force,
            remove_volumes
        );
        Ok(())
    }

    pub async fn pause(&self, container_id: &str) -> Result<(), ContainerError> {
        tracing::info!("Pausing container: {}", container_id);
        Ok(())
    }

    pub async fn unpause(&self, container_id: &str) -> Result<(), ContainerError> {
        tracing::info!("Unpausing container: {}", container_id);
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

    pub async fn wait(
        &self,
        container_id: &str,
        condition: &str,
    ) -> Result<WaitResult, ContainerError> {
        tracing::info!(
            "Waiting for container: {} with condition: {}",
            container_id,
            condition
        );
        Ok(WaitResult {
            status_code: 0,
            error: None,
        })
    }

    pub async fn kill(&self, container_id: &str, signal: &str) -> Result<(), ContainerError> {
        tracing::info!(
            "Killing container: {} with signal: {}",
            container_id,
            signal
        );
        Ok(())
    }

    pub async fn rename(
        &self,
        container_id: &str,
        new_name: &str,
    ) -> Result<(), ContainerError> {
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
}
