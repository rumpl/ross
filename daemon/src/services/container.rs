use ross_container::{
    AttachInput, ContainerService, CreateContainerParams, ExecConfig, GetLogsParams, InputEvent,
    ListContainersParams, OutputEvent, StatsParams,
};
use ross_core::container_service_server::ContainerService as GrpcContainerService;
use ross_core::{
    AttachOutput, AttachRequest, CreateContainerRequest, CreateContainerResponse, ExecOutput,
    ExecRequest, ExecResponse, ExecStartRequest, GetLogsRequest, InspectContainerRequest,
    InspectContainerResponse, InteractiveInput, InteractiveOutput, KillContainerRequest,
    KillContainerResponse, ListContainersRequest, ListContainersResponse, LogEntry,
    PauseContainerRequest, PauseContainerResponse, RemoveContainerRequest, RemoveContainerResponse,
    RenameContainerRequest, RenameContainerResponse, RestartContainerRequest,
    RestartContainerResponse, StartContainerRequest, StartContainerResponse, StatsRequest,
    StatsResponse, StopContainerRequest, StopContainerResponse, UnpauseContainerRequest,
    UnpauseContainerResponse, WaitContainerOutput, WaitContainerRequest,
};
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status, Streaming};

type StreamResult<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

pub struct ContainerServiceGrpc {
    service: Arc<ContainerService>,
}

impl ContainerServiceGrpc {
    pub fn new(service: Arc<ContainerService>) -> Self {
        Self { service }
    }
}

#[tonic::async_trait]
impl GrpcContainerService for ContainerServiceGrpc {
    async fn create_container(
        &self,
        request: Request<CreateContainerRequest>,
    ) -> Result<Response<CreateContainerResponse>, Status> {
        let req = request.into_inner();

        let params = CreateContainerParams {
            name: if req.name.is_empty() {
                None
            } else {
                Some(req.name)
            },
            config: req
                .config
                .map(container_config_from_grpc)
                .unwrap_or_default(),
            host_config: req
                .host_config
                .map(host_config_from_grpc)
                .unwrap_or_default(),
            networking_config: req
                .networking_config
                .map(networking_config_from_grpc)
                .unwrap_or_default(),
        };

        let result = self.service.create(params).await.map_err(into_status)?;

        Ok(Response::new(CreateContainerResponse {
            id: result.id,
            warnings: result.warnings,
        }))
    }

    async fn start_container(
        &self,
        request: Request<StartContainerRequest>,
    ) -> Result<Response<StartContainerResponse>, Status> {
        let req = request.into_inner();

        if req.container_id.is_empty() {
            return Err(Status::invalid_argument("container_id is required"));
        }

        self.service
            .start(&req.container_id)
            .await
            .map_err(into_status)?;

        Ok(Response::new(StartContainerResponse {}))
    }

    async fn stop_container(
        &self,
        request: Request<StopContainerRequest>,
    ) -> Result<Response<StopContainerResponse>, Status> {
        let req = request.into_inner();

        if req.container_id.is_empty() {
            return Err(Status::invalid_argument("container_id is required"));
        }

        self.service
            .stop(&req.container_id, req.timeout)
            .await
            .map_err(into_status)?;

        Ok(Response::new(StopContainerResponse {}))
    }

    async fn restart_container(
        &self,
        request: Request<RestartContainerRequest>,
    ) -> Result<Response<RestartContainerResponse>, Status> {
        let req = request.into_inner();

        if req.container_id.is_empty() {
            return Err(Status::invalid_argument("container_id is required"));
        }

        self.service
            .restart(&req.container_id, req.timeout)
            .await
            .map_err(into_status)?;

        Ok(Response::new(RestartContainerResponse {}))
    }

    async fn list_containers(
        &self,
        request: Request<ListContainersRequest>,
    ) -> Result<Response<ListContainersResponse>, Status> {
        let req = request.into_inner();

        let params = ListContainersParams {
            all: req.all,
            limit: req.limit,
            size: req.size,
            filters: req.filters,
        };

        let containers = self.service.list(params).await.map_err(into_status)?;

        Ok(Response::new(ListContainersResponse {
            containers: containers.into_iter().map(container_to_grpc).collect(),
        }))
    }

    async fn inspect_container(
        &self,
        request: Request<InspectContainerRequest>,
    ) -> Result<Response<InspectContainerResponse>, Status> {
        let req = request.into_inner();

        if req.container_id.is_empty() {
            return Err(Status::invalid_argument("container_id is required"));
        }

        let inspection = self
            .service
            .inspect(&req.container_id)
            .await
            .map_err(into_status)?;

        Ok(Response::new(inspection_to_grpc(inspection)))
    }

    async fn remove_container(
        &self,
        request: Request<RemoveContainerRequest>,
    ) -> Result<Response<RemoveContainerResponse>, Status> {
        let req = request.into_inner();

        if req.container_id.is_empty() {
            return Err(Status::invalid_argument("container_id is required"));
        }

        self.service
            .remove(&req.container_id, req.force, req.remove_volumes)
            .await
            .map_err(into_status)?;

        Ok(Response::new(RemoveContainerResponse {}))
    }

    async fn pause_container(
        &self,
        request: Request<PauseContainerRequest>,
    ) -> Result<Response<PauseContainerResponse>, Status> {
        let req = request.into_inner();

        if req.container_id.is_empty() {
            return Err(Status::invalid_argument("container_id is required"));
        }

        self.service
            .pause(&req.container_id)
            .await
            .map_err(into_status)?;

        Ok(Response::new(PauseContainerResponse {}))
    }

    async fn unpause_container(
        &self,
        request: Request<UnpauseContainerRequest>,
    ) -> Result<Response<UnpauseContainerResponse>, Status> {
        let req = request.into_inner();

        if req.container_id.is_empty() {
            return Err(Status::invalid_argument("container_id is required"));
        }

        self.service
            .unpause(&req.container_id)
            .await
            .map_err(into_status)?;

        Ok(Response::new(UnpauseContainerResponse {}))
    }

    type GetLogsStream = StreamResult<LogEntry>;

    async fn get_logs(
        &self,
        request: Request<GetLogsRequest>,
    ) -> Result<Response<Self::GetLogsStream>, Status> {
        let req = request.into_inner();

        if req.container_id.is_empty() {
            return Err(Status::invalid_argument("container_id is required"));
        }

        let params = GetLogsParams {
            container_id: req.container_id,
            follow: req.follow,
            stdout: req.stdout,
            stderr: req.stderr,
            since: req.since,
            until: req.until,
            timestamps: req.timestamps,
            tail: req.tail,
        };

        let stream = self.service.get_logs(params);
        let output = stream.map(|result| result.map(log_entry_to_grpc).map_err(into_status));

        Ok(Response::new(Box::pin(output)))
    }

    async fn exec(&self, request: Request<ExecRequest>) -> Result<Response<ExecResponse>, Status> {
        let req = request.into_inner();

        if req.container_id.is_empty() {
            return Err(Status::invalid_argument("container_id is required"));
        }

        let config = req.config.map(exec_config_from_grpc).unwrap_or_default();

        let exec_id = self
            .service
            .exec_create(&req.container_id, config)
            .await
            .map_err(into_status)?;

        Ok(Response::new(ExecResponse { exec_id }))
    }

    type ExecStartStream = StreamResult<ExecOutput>;

    async fn exec_start(
        &self,
        request: Request<ExecStartRequest>,
    ) -> Result<Response<Self::ExecStartStream>, Status> {
        let req = request.into_inner();

        if req.exec_id.is_empty() {
            return Err(Status::invalid_argument("exec_id is required"));
        }

        let stream = self.service.exec_start(&req.exec_id);
        let output = stream.map(|result| result.map(exec_output_to_grpc).map_err(into_status));

        Ok(Response::new(Box::pin(output)))
    }

    type AttachStream = StreamResult<AttachOutput>;

    async fn attach(
        &self,
        request: Request<Streaming<AttachRequest>>,
    ) -> Result<Response<Self::AttachStream>, Status> {
        let input_stream = request.into_inner();

        let mapped_input = input_stream.map(|result| {
            result
                .map(|req| AttachInput {
                    container_id: req.container_id,
                    stream: req.stream,
                    stdin: req.stdin,
                    stdout: req.stdout,
                    stderr: req.stderr,
                    detach_keys: req.detach_keys,
                    logs: req.logs,
                    input: req.input,
                })
                .map_err(|e| ross_container::ContainerError::InvalidArgument(e.to_string()))
        });

        let stream = self.service.attach(mapped_input);
        let output = stream.map(|result| result.map(attach_output_to_grpc).map_err(into_status));

        Ok(Response::new(Box::pin(output)))
    }

    type WaitStream = StreamResult<WaitContainerOutput>;

    async fn wait(
        &self,
        request: Request<WaitContainerRequest>,
    ) -> Result<Response<Self::WaitStream>, Status> {
        let req = request.into_inner();

        if req.container_id.is_empty() {
            return Err(Status::invalid_argument("container_id is required"));
        }

        let stream = self.service.wait_streaming(&req.container_id);
        let output = stream.map(|result| {
            result
                .map(|event| match event {
                    OutputEvent::Stdout(data) => WaitContainerOutput {
                        output: Some(ross_core::wait_container_output::Output::Data(
                            ross_core::OutputData {
                                stream: "stdout".to_string(),
                                data,
                            },
                        )),
                    },
                    OutputEvent::Stderr(data) => WaitContainerOutput {
                        output: Some(ross_core::wait_container_output::Output::Data(
                            ross_core::OutputData {
                                stream: "stderr".to_string(),
                                data,
                            },
                        )),
                    },
                    OutputEvent::Exit(result) => WaitContainerOutput {
                        output: Some(ross_core::wait_container_output::Output::Exit(
                            ross_core::ExitResult {
                                status_code: result.status_code,
                                error: result
                                    .error
                                    .map(|msg| ross_core::WaitError { message: msg }),
                            },
                        )),
                    },
                })
                .map_err(into_status)
        });

        Ok(Response::new(Box::pin(output)))
    }

    async fn kill(
        &self,
        request: Request<KillContainerRequest>,
    ) -> Result<Response<KillContainerResponse>, Status> {
        let req = request.into_inner();

        if req.container_id.is_empty() {
            return Err(Status::invalid_argument("container_id is required"));
        }

        self.service
            .kill(&req.container_id, &req.signal)
            .await
            .map_err(into_status)?;

        Ok(Response::new(KillContainerResponse {}))
    }

    async fn rename(
        &self,
        request: Request<RenameContainerRequest>,
    ) -> Result<Response<RenameContainerResponse>, Status> {
        let req = request.into_inner();

        if req.container_id.is_empty() {
            return Err(Status::invalid_argument("container_id is required"));
        }

        if req.new_name.is_empty() {
            return Err(Status::invalid_argument("new_name is required"));
        }

        self.service
            .rename(&req.container_id, &req.new_name)
            .await
            .map_err(into_status)?;

        Ok(Response::new(RenameContainerResponse {}))
    }

    type StatsStream = StreamResult<StatsResponse>;

    async fn stats(
        &self,
        request: Request<StatsRequest>,
    ) -> Result<Response<Self::StatsStream>, Status> {
        let req = request.into_inner();

        if req.container_id.is_empty() {
            return Err(Status::invalid_argument("container_id is required"));
        }

        let params = StatsParams {
            container_id: req.container_id,
            stream: req.stream,
            one_shot: req.one_shot,
        };

        let stream = self.service.stats(params);
        let output = stream.map(|result| result.map(stats_to_grpc).map_err(into_status));

        Ok(Response::new(Box::pin(output)))
    }

    type RunInteractiveStream = StreamResult<InteractiveOutput>;

    async fn run_interactive(
        &self,
        request: Request<Streaming<InteractiveInput>>,
    ) -> Result<Response<Self::RunInteractiveStream>, Status> {
        let mut input_stream = request.into_inner();

        // First message must be InteractiveStart
        let first = input_stream
            .next()
            .await
            .ok_or_else(|| Status::invalid_argument("Expected start message"))?
            .map_err(|e| Status::internal(e.to_string()))?;

        let start = match first.input {
            Some(ross_core::interactive_input::Input::Start(s)) => s,
            _ => {
                return Err(Status::invalid_argument(
                    "First message must be InteractiveStart",
                ))
            }
        };

        if start.container_id.is_empty() {
            return Err(Status::invalid_argument("container_id is required"));
        }

        let (input_tx, mut output_stream) = self
            .service
            .run_interactive(start.container_id.clone(), start.tty)
            .await
            .map_err(into_status)?;

        // Spawn task to forward input from gRPC stream to container
        tokio::spawn(async move {
            tracing::debug!("Input forwarding task started");
            while let Some(result) = input_stream.next().await {
                match result {
                    Ok(msg) => {
                        let event = match msg.input {
                            Some(ross_core::interactive_input::Input::Stdin(data)) => {
                                tracing::debug!("Received stdin from client: {} bytes", data.len());
                                InputEvent::Stdin(data)
                            }
                            Some(ross_core::interactive_input::Input::Resize(size)) => {
                                InputEvent::Resize {
                                    width: size.width as u16,
                                    height: size.height as u16,
                                }
                            }
                            Some(ross_core::interactive_input::Input::Start(_)) => {
                                tracing::warn!("Unexpected start message after session started");
                                continue;
                            }
                            None => continue,
                        };
                        if input_tx.send(event).await.is_err() {
                            tracing::debug!("Input channel closed");
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Error receiving input: {}", e);
                        break;
                    }
                }
            }
            tracing::debug!("Input forwarding task ended");
        });

        // Map container output events to gRPC messages
        let grpc_output = async_stream::stream! {
            tracing::debug!("gRPC output stream started");
            while let Some(result) = output_stream.next().await {
                tracing::debug!("Got output from container service");
                let grpc_msg = match result {
                    Ok(OutputEvent::Stdout(data)) => {
                        tracing::debug!("Sending {} bytes stdout to client", data.len());
                        InteractiveOutput {
                            output: Some(ross_core::interactive_output::Output::Data(
                                ross_core::OutputData {
                                    stream: "stdout".to_string(),
                                    data,
                                },
                            )),
                        }
                    }
                    Ok(OutputEvent::Stderr(data)) => {
                        tracing::debug!("Sending {} bytes stderr to client", data.len());
                        InteractiveOutput {
                            output: Some(ross_core::interactive_output::Output::Data(
                                ross_core::OutputData {
                                    stream: "stderr".to_string(),
                                    data,
                                },
                            )),
                        }
                    }
                    Ok(OutputEvent::Exit(result)) => InteractiveOutput {
                        output: Some(ross_core::interactive_output::Output::Exit(
                            ross_core::ExitResult {
                                status_code: result.status_code,
                                error: result.error.map(|msg| ross_core::WaitError { message: msg }),
                            },
                        )),
                    },
                    Err(e) => {
                        tracing::error!("Container output error: {}", e);
                        break;
                    }
                };
                yield Ok(grpc_msg);
            }
        };

        Ok(Response::new(Box::pin(grpc_output)))
    }
}

fn into_status(e: ross_container::ContainerError) -> Status {
    match e {
        ross_container::ContainerError::NotFound(_) => Status::not_found(e.to_string()),
        ross_container::ContainerError::AlreadyExists(_) => Status::already_exists(e.to_string()),
        ross_container::ContainerError::NotRunning(_)
        | ross_container::ContainerError::AlreadyRunning(_) => {
            Status::failed_precondition(e.to_string())
        }
        ross_container::ContainerError::ExecNotFound(_) => Status::not_found(e.to_string()),
        ross_container::ContainerError::InvalidArgument(_) => {
            Status::invalid_argument(e.to_string())
        }
        ross_container::ContainerError::ImageNotFound(_) => Status::not_found(e.to_string()),
        ross_container::ContainerError::Io(_)
        | ross_container::ContainerError::Shim(_)
        | ross_container::ContainerError::Snapshotter(_)
        | ross_container::ContainerError::Store(_) => Status::internal(e.to_string()),
    }
}

fn container_config_from_grpc(c: ross_core::ContainerConfig) -> ross_container::ContainerConfig {
    ross_container::ContainerConfig {
        hostname: c.hostname,
        domainname: c.domainname,
        user: c.user,
        attach_stdin: c.attach_stdin,
        attach_stdout: c.attach_stdout,
        attach_stderr: c.attach_stderr,
        exposed_ports: c.exposed_ports,
        tty: c.tty,
        open_stdin: c.open_stdin,
        stdin_once: c.stdin_once,
        env: c.env,
        cmd: c.cmd,
        entrypoint: c.entrypoint,
        image: c.image,
        labels: c.labels,
        working_dir: c.working_dir,
        network_disabled: c.network_disabled,
        mac_address: c.mac_address,
        stop_signal: c.stop_signal,
        stop_timeout: c.stop_timeout,
        shell: c.shell,
    }
}

fn host_config_from_grpc(h: ross_core::HostConfig) -> ross_container::HostConfig {
    ross_container::HostConfig {
        binds: h.binds,
        network_mode: h.network_mode,
        port_bindings: h
            .port_bindings
            .into_iter()
            .map(port_binding_from_grpc)
            .collect(),
        auto_remove: h.auto_remove,
        privileged: h.privileged,
        publish_all_ports: h.publish_all_ports,
        readonly_rootfs: h.readonly_rootfs,
    }
}

fn port_binding_from_grpc(p: ross_core::PortBinding) -> ross_container::PortBinding {
    ross_container::PortBinding {
        host_ip: p.host_ip,
        host_port: p.host_port,
        container_port: p.container_port,
        protocol: p.protocol,
    }
}

fn networking_config_from_grpc(n: ross_core::NetworkingConfig) -> ross_container::NetworkingConfig {
    ross_container::NetworkingConfig {
        endpoints_config: n
            .endpoints_config
            .into_iter()
            .map(|(k, v)| (k, endpoint_config_from_grpc(v)))
            .collect(),
    }
}

fn endpoint_config_from_grpc(e: ross_core::EndpointConfig) -> ross_container::EndpointConfig {
    ross_container::EndpointConfig {
        network_id: e.network_id,
        endpoint_id: e.endpoint_id,
        gateway: e.gateway,
        ip_address: e.ip_address,
        ip_prefix_len: e.ip_prefix_len,
        mac_address: e.mac_address,
        aliases: e.aliases,
    }
}

fn exec_config_from_grpc(e: ross_core::ExecConfig) -> ExecConfig {
    ExecConfig {
        attach_stdin: e.attach_stdin,
        attach_stdout: e.attach_stdout,
        attach_stderr: e.attach_stderr,
        detach_keys: e.detach_keys,
        tty: e.tty,
        env: e.env,
        cmd: e.cmd,
        privileged: e.privileged,
        user: e.user,
        working_dir: e.working_dir,
    }
}

fn container_to_grpc(c: ross_container::Container) -> ross_core::Container {
    ross_core::Container {
        id: c.id,
        names: c.names,
        image: c.image,
        image_id: c.image_id,
        command: c.command,
        created: c.created,
        state: c.state,
        status: c.status,
        ports: c.ports.into_iter().map(port_binding_to_grpc).collect(),
        labels: c.labels,
        size_rw: c.size_rw,
        size_root_fs: c.size_root_fs,
        host_config: None,
        network_settings: None,
        mounts: vec![],
    }
}

fn port_binding_to_grpc(p: ross_container::PortBinding) -> ross_core::PortBinding {
    ross_core::PortBinding {
        host_ip: p.host_ip,
        host_port: p.host_port,
        container_port: p.container_port,
        protocol: p.protocol,
    }
}

fn inspection_to_grpc(i: ross_container::ContainerInspection) -> InspectContainerResponse {
    InspectContainerResponse {
        container: Some(container_to_grpc(i.container)),
        state: Some(container_state_to_grpc(i.state)),
        path: i.path,
        args: i.args,
        resolv_conf_path: i.resolv_conf_path,
        hostname_path: i.hostname_path,
        hosts_path: i.hosts_path,
        log_path: i.log_path,
        name: i.name,
        restart_count: i.restart_count,
        driver: i.driver,
        platform: i.platform,
        mount_label: i.mount_label,
        process_label: i.process_label,
        app_armor_profile: i.app_armor_profile,
        exec_ids: i.exec_ids,
        config: Some(container_config_to_grpc(i.config)),
        host_config: Some(host_config_to_grpc(i.host_config)),
        graph_driver: None,
        network_settings: None,
    }
}

fn container_state_to_grpc(s: ross_container::ContainerState) -> ross_core::ContainerState {
    ross_core::ContainerState {
        status: s.status,
        running: s.running,
        paused: s.paused,
        restarting: s.restarting,
        oom_killed: s.oom_killed,
        dead: s.dead,
        pid: s.pid,
        exit_code: s.exit_code,
        error: s.error,
        started_at: s.started_at,
        finished_at: s.finished_at,
        health: None,
    }
}

fn container_config_to_grpc(c: ross_container::ContainerConfig) -> ross_core::ContainerConfig {
    ross_core::ContainerConfig {
        hostname: c.hostname,
        domainname: c.domainname,
        user: c.user,
        attach_stdin: c.attach_stdin,
        attach_stdout: c.attach_stdout,
        attach_stderr: c.attach_stderr,
        exposed_ports: c.exposed_ports,
        tty: c.tty,
        open_stdin: c.open_stdin,
        stdin_once: c.stdin_once,
        env: c.env,
        cmd: c.cmd,
        entrypoint: c.entrypoint,
        image: c.image,
        labels: c.labels,
        volumes: Default::default(),
        working_dir: c.working_dir,
        network_disabled: c.network_disabled,
        mac_address: c.mac_address,
        stop_signal: c.stop_signal,
        stop_timeout: c.stop_timeout,
        shell: c.shell,
        healthcheck: None,
    }
}

fn host_config_to_grpc(h: ross_container::HostConfig) -> ross_core::HostConfig {
    ross_core::HostConfig {
        binds: h.binds,
        network_mode: h.network_mode,
        port_bindings: h
            .port_bindings
            .into_iter()
            .map(port_binding_to_grpc)
            .collect(),
        auto_remove: h.auto_remove,
        privileged: h.privileged,
        publish_all_ports: h.publish_all_ports,
        readonly_rootfs: h.readonly_rootfs,
        ..Default::default()
    }
}

fn log_entry_to_grpc(l: ross_container::LogEntry) -> LogEntry {
    LogEntry {
        timestamp: Some(l.timestamp),
        stream: l.stream,
        message: l.message,
    }
}

fn exec_output_to_grpc(e: ross_container::ExecOutput) -> ExecOutput {
    ExecOutput {
        stream: e.stream,
        data: e.data,
    }
}

fn attach_output_to_grpc(a: ross_container::AttachOutput) -> AttachOutput {
    AttachOutput {
        stream: a.stream,
        data: a.data,
    }
}

fn stats_to_grpc(s: ross_container::ContainerStats) -> StatsResponse {
    StatsResponse {
        read: s.read,
        preread: s.preread,
        pids_stats: None,
        blkio_stats: None,
        num_procs: s.num_procs,
        storage_stats: None,
        cpu_stats: s.cpu_stats.map(cpu_stats_to_grpc),
        precpu_stats: s.precpu_stats.map(cpu_stats_to_grpc),
        memory_stats: s.memory_stats.map(memory_stats_to_grpc),
        networks: s
            .networks
            .into_iter()
            .map(|(k, v)| (k, network_stats_to_grpc(v)))
            .collect(),
    }
}

fn cpu_stats_to_grpc(c: ross_container::CpuStats) -> ross_core::CpuStats {
    ross_core::CpuStats {
        cpu_usage: c.cpu_usage.map(cpu_usage_to_grpc),
        system_cpu_usage: c.system_cpu_usage,
        online_cpus: c.online_cpus,
        throttling_data: None,
    }
}

fn cpu_usage_to_grpc(c: ross_container::CpuUsage) -> ross_core::CpuUsage {
    ross_core::CpuUsage {
        total_usage: c.total_usage,
        percpu_usage: c.percpu_usage,
        usage_in_kernelmode: c.usage_in_kernelmode,
        usage_in_usermode: c.usage_in_usermode,
    }
}

fn memory_stats_to_grpc(m: ross_container::MemoryStats) -> ross_core::MemoryStats {
    ross_core::MemoryStats {
        usage: m.usage,
        max_usage: m.max_usage,
        stats: m.stats,
        failcnt: m.failcnt,
        limit: m.limit,
        commit: m.commit,
        commit_peak: m.commit_peak,
        private_working_set: m.private_working_set,
    }
}

fn network_stats_to_grpc(n: ross_container::NetworkStats) -> ross_core::NetworkStats {
    ross_core::NetworkStats {
        rx_bytes: n.rx_bytes,
        rx_packets: n.rx_packets,
        rx_errors: n.rx_errors,
        rx_dropped: n.rx_dropped,
        tx_bytes: n.tx_bytes,
        tx_packets: n.tx_packets,
        tx_errors: n.tx_errors,
        tx_dropped: n.tx_dropped,
    }
}
