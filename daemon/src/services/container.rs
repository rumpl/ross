use async_stream::stream;
use prost_types::Timestamp;
use ross_core::container_service_server::ContainerService;
use ross_core::{
    AttachOutput, AttachRequest, CpuStats, CpuUsage, CreateContainerRequest,
    CreateContainerResponse, ExecOutput, ExecRequest, ExecResponse, ExecStartRequest,
    GetLogsRequest, InspectContainerRequest, InspectContainerResponse, KillContainerRequest,
    KillContainerResponse, ListContainersRequest, ListContainersResponse, LogEntry, MemoryStats,
    PauseContainerRequest, PauseContainerResponse, RemoveContainerRequest, RemoveContainerResponse,
    RenameContainerRequest, RenameContainerResponse, RestartContainerRequest,
    RestartContainerResponse, StartContainerRequest, StartContainerResponse, StatsRequest,
    StatsResponse, StopContainerRequest, StopContainerResponse, UnpauseContainerRequest,
    UnpauseContainerResponse, WaitContainerRequest, WaitContainerResponse,
};
use std::pin::Pin;
use std::time::SystemTime;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status, Streaming};

type StreamResult<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

fn now_timestamp() -> Option<Timestamp> {
    Some(Timestamp::from(SystemTime::now()))
}

#[derive(Default)]
pub struct ContainerServiceImpl;

#[tonic::async_trait]
impl ContainerService for ContainerServiceImpl {
    async fn create_container(
        &self,
        request: Request<CreateContainerRequest>,
    ) -> Result<Response<CreateContainerResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("Creating container with name: {:?}", req.name);
        Ok(Response::new(CreateContainerResponse {
            id: "stub-container-id".to_string(),
            warnings: vec![],
        }))
    }

    async fn start_container(
        &self,
        request: Request<StartContainerRequest>,
    ) -> Result<Response<StartContainerResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("Starting container: {}", req.container_id);
        Ok(Response::new(StartContainerResponse {}))
    }

    async fn stop_container(
        &self,
        request: Request<StopContainerRequest>,
    ) -> Result<Response<StopContainerResponse>, Status> {
        let req = request.into_inner();
        tracing::info!(
            "Stopping container: {} with timeout: {}",
            req.container_id,
            req.timeout
        );
        Ok(Response::new(StopContainerResponse {}))
    }

    async fn restart_container(
        &self,
        request: Request<RestartContainerRequest>,
    ) -> Result<Response<RestartContainerResponse>, Status> {
        let req = request.into_inner();
        tracing::info!(
            "Restarting container: {} with timeout: {}",
            req.container_id,
            req.timeout
        );
        Ok(Response::new(RestartContainerResponse {}))
    }

    async fn list_containers(
        &self,
        request: Request<ListContainersRequest>,
    ) -> Result<Response<ListContainersResponse>, Status> {
        let req = request.into_inner();
        tracing::info!(
            "Listing containers (all: {}, limit: {})",
            req.all,
            req.limit
        );
        Ok(Response::new(ListContainersResponse { containers: vec![] }))
    }

    async fn inspect_container(
        &self,
        request: Request<InspectContainerRequest>,
    ) -> Result<Response<InspectContainerResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("Inspecting container: {}", req.container_id);
        Ok(Response::new(InspectContainerResponse {
            container: None,
            state: None,
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
            config: None,
            host_config: None,
            graph_driver: None,
            network_settings: None,
        }))
    }

    async fn remove_container(
        &self,
        request: Request<RemoveContainerRequest>,
    ) -> Result<Response<RemoveContainerResponse>, Status> {
        let req = request.into_inner();
        tracing::info!(
            "Removing container: {} (force: {}, volumes: {})",
            req.container_id,
            req.force,
            req.remove_volumes
        );
        Ok(Response::new(RemoveContainerResponse {}))
    }

    async fn pause_container(
        &self,
        request: Request<PauseContainerRequest>,
    ) -> Result<Response<PauseContainerResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("Pausing container: {}", req.container_id);
        Ok(Response::new(PauseContainerResponse {}))
    }

    async fn unpause_container(
        &self,
        request: Request<UnpauseContainerRequest>,
    ) -> Result<Response<UnpauseContainerResponse>, Status> {
        let req = request.into_inner();
        tracing::info!("Unpausing container: {}", req.container_id);
        Ok(Response::new(UnpauseContainerResponse {}))
    }

    type GetLogsStream = StreamResult<LogEntry>;

    async fn get_logs(
        &self,
        request: Request<GetLogsRequest>,
    ) -> Result<Response<Self::GetLogsStream>, Status> {
        let req = request.into_inner();
        tracing::info!(
            "Getting logs for container: {} (follow: {})",
            req.container_id,
            req.follow
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

        Ok(Response::new(Box::pin(output)))
    }

    async fn exec(&self, request: Request<ExecRequest>) -> Result<Response<ExecResponse>, Status> {
        let req = request.into_inner();
        let cmd = req.config.as_ref().map(|c| &c.cmd);
        tracing::info!(
            "Creating exec instance in container: {} with cmd: {:?}",
            req.container_id,
            cmd
        );
        Ok(Response::new(ExecResponse {
            exec_id: "stub-exec-id".to_string(),
        }))
    }

    type ExecStartStream = StreamResult<ExecOutput>;

    async fn exec_start(
        &self,
        request: Request<ExecStartRequest>,
    ) -> Result<Response<Self::ExecStartStream>, Status> {
        let req = request.into_inner();
        tracing::info!("Starting exec: {}", req.exec_id);

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

        Ok(Response::new(Box::pin(output)))
    }

    type AttachStream = StreamResult<AttachOutput>;

    async fn attach(
        &self,
        request: Request<Streaming<AttachRequest>>,
    ) -> Result<Response<Self::AttachStream>, Status> {
        tracing::info!("Attaching to container");

        let mut input_stream = request.into_inner();

        let output = stream! {
            while let Some(result) = input_stream.next().await {
                match result {
                    Ok(attach_req) => {
                        tracing::info!(
                            "Received attach input for container: {}, {} bytes",
                            attach_req.container_id,
                            attach_req.input.len()
                        );
                        yield Ok(AttachOutput {
                            stream: "stdout".to_string(),
                            data: attach_req.input,
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Error receiving attach input: {}", e);
                        break;
                    }
                }
            }
        };

        Ok(Response::new(Box::pin(output)))
    }

    async fn wait(
        &self,
        request: Request<WaitContainerRequest>,
    ) -> Result<Response<WaitContainerResponse>, Status> {
        let req = request.into_inner();
        tracing::info!(
            "Waiting for container: {} with condition: {}",
            req.container_id,
            req.condition
        );
        Ok(Response::new(WaitContainerResponse {
            status_code: 0,
            error: None,
        }))
    }

    async fn kill(
        &self,
        request: Request<KillContainerRequest>,
    ) -> Result<Response<KillContainerResponse>, Status> {
        let req = request.into_inner();
        tracing::info!(
            "Killing container: {} with signal: {}",
            req.container_id,
            req.signal
        );
        Ok(Response::new(KillContainerResponse {}))
    }

    async fn rename(
        &self,
        request: Request<RenameContainerRequest>,
    ) -> Result<Response<RenameContainerResponse>, Status> {
        let req = request.into_inner();
        tracing::info!(
            "Renaming container: {} to: {}",
            req.container_id,
            req.new_name
        );
        Ok(Response::new(RenameContainerResponse {}))
    }

    type StatsStream = StreamResult<StatsResponse>;

    async fn stats(
        &self,
        request: Request<StatsRequest>,
    ) -> Result<Response<Self::StatsStream>, Status> {
        let req = request.into_inner();
        tracing::info!(
            "Getting stats for container: {} (stream: {})",
            req.container_id,
            req.stream
        );

        let output = stream! {
            for i in 0..3 {
                yield Ok(StatsResponse {
                    read: now_timestamp(),
                    preread: now_timestamp(),
                    pids_stats: None,
                    blkio_stats: None,
                    num_procs: 1,
                    storage_stats: None,
                    cpu_stats: Some(CpuStats {
                        cpu_usage: Some(CpuUsage {
                            total_usage: 1000000 * (i + 1),
                            percpu_usage: vec![500000 * (i + 1)],
                            usage_in_kernelmode: 100000 * (i + 1),
                            usage_in_usermode: 900000 * (i + 1),
                        }),
                        system_cpu_usage: 10000000000,
                        online_cpus: 4,
                        throttling_data: None,
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

        Ok(Response::new(Box::pin(output)))
    }
}
