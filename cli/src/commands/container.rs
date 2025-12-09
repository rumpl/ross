use clap::Subcommand;
use ross_core::ross::container_service_client::ContainerServiceClient;
use ross_core::ross::{
    AttachRequest, ContainerConfig, CreateContainerRequest, ExecConfig, ExecRequest,
    ExecStartRequest, GetLogsRequest, HostConfig, InspectContainerRequest, KillContainerRequest,
    ListContainersRequest, PauseContainerRequest, PortBinding, RemoveContainerRequest,
    RenameContainerRequest, RestartContainerRequest, StartContainerRequest, StatsRequest,
    StopContainerRequest, UnpauseContainerRequest, WaitContainerRequest,
};
use tokio_stream::StreamExt;

use crate::utils::{format_size, format_timestamp};

#[derive(Subcommand)]
pub enum ContainerCommands {
    /// Create a new container
    Create {
        /// Image to create the container from
        image: String,

        /// Assign a name to the container
        #[arg(long)]
        name: Option<String>,

        /// Set environment variables (KEY=VAL)
        #[arg(long, short)]
        env: Vec<String>,

        /// Publish a container's port(s) to the host (HOST:CONTAINER)
        #[arg(long = "publish", short = 'p')]
        publish: Vec<String>,

        /// Bind mount a volume (SRC:DST)
        #[arg(long, short)]
        volume: Vec<String>,
    },
    /// Start one or more stopped containers
    Start {
        /// Container ID or name
        container_id: String,
    },
    /// Stop one or more running containers
    Stop {
        /// Container ID or name
        container_id: String,

        /// Seconds to wait for stop before killing it
        #[arg(long, short, default_value_t = 10)]
        timeout: i32,
    },
    /// Restart one or more containers
    Restart {
        /// Container ID or name
        container_id: String,

        /// Seconds to wait for stop before killing it
        #[arg(long, short, default_value_t = 10)]
        timeout: i32,
    },
    /// List containers
    #[command(visible_alias = "ps")]
    List {
        /// Show all containers (default shows just running)
        #[arg(long, short)]
        all: bool,

        /// Show n last created containers (includes all states)
        #[arg(long, short)]
        limit: Option<i32>,
    },
    /// Display detailed information on one or more containers
    Inspect {
        /// Container ID or name
        container_id: String,
    },
    /// Remove one or more containers
    #[command(visible_alias = "rm")]
    Remove {
        /// Container ID or name
        container_id: String,

        /// Force the removal of a running container
        #[arg(long, short)]
        force: bool,

        /// Remove anonymous volumes associated with the container
        #[arg(long, short = 'v')]
        volumes: bool,
    },
    /// Pause all processes within one or more containers
    Pause {
        /// Container ID or name
        container_id: String,
    },
    /// Unpause all processes within one or more containers
    Unpause {
        /// Container ID or name
        container_id: String,
    },
    /// Fetch the logs of a container
    Logs {
        /// Container ID or name
        container_id: String,

        /// Follow log output
        #[arg(long, short)]
        follow: bool,

        /// Number of lines to show from the end of the logs
        #[arg(long, default_value = "all")]
        tail: String,

        /// Show timestamps
        #[arg(long, short)]
        timestamps: bool,
    },
    /// Run a command in a running container
    Exec {
        /// Container ID or name
        container_id: String,

        /// Allocate a pseudo-TTY
        #[arg(long, short)]
        tty: bool,

        /// Keep STDIN open even if not attached
        #[arg(long, short)]
        interactive: bool,

        /// Command to execute
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Attach local standard input, output, and error streams to a running container
    Attach {
        /// Container ID or name
        container_id: String,
    },
    /// Block until one or more containers stop, then print their exit codes
    Wait {
        /// Container ID or name
        container_id: String,
    },
    /// Kill one or more running containers
    Kill {
        /// Container ID or name
        container_id: String,

        /// Signal to send to the container
        #[arg(long, short, default_value = "SIGKILL")]
        signal: String,
    },
    /// Rename a container
    Rename {
        /// Container ID or name
        container_id: String,

        /// New name for the container
        new_name: String,
    },
    /// Display a live stream of container(s) resource usage statistics
    Stats {
        /// Container ID or name
        container_id: String,

        /// Disable streaming stats and only pull the first result
        #[arg(long)]
        no_stream: bool,
    },
}

pub async fn handle_container_command(
    addr: &str,
    cmd: ContainerCommands,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut client = ContainerServiceClient::connect(addr.to_string())
        .await
        .map_err(|e| {
            format!(
                "Failed to connect to daemon at {}: {}. Is the daemon running?",
                addr, e
            )
        })?;

    match cmd {
        ContainerCommands::Create {
            image,
            name,
            env,
            publish,
            volume,
        } => {
            container_create(&mut client, &image, name, env, publish, volume).await?;
        }
        ContainerCommands::Start { container_id } => {
            container_start(&mut client, &container_id).await?;
        }
        ContainerCommands::Stop {
            container_id,
            timeout,
        } => {
            container_stop(&mut client, &container_id, timeout).await?;
        }
        ContainerCommands::Restart {
            container_id,
            timeout,
        } => {
            container_restart(&mut client, &container_id, timeout).await?;
        }
        ContainerCommands::List { all, limit } => {
            container_list(&mut client, all, limit).await?;
        }
        ContainerCommands::Inspect { container_id } => {
            container_inspect(&mut client, &container_id).await?;
        }
        ContainerCommands::Remove {
            container_id,
            force,
            volumes,
        } => {
            container_remove(&mut client, &container_id, force, volumes).await?;
        }
        ContainerCommands::Pause { container_id } => {
            container_pause(&mut client, &container_id).await?;
        }
        ContainerCommands::Unpause { container_id } => {
            container_unpause(&mut client, &container_id).await?;
        }
        ContainerCommands::Logs {
            container_id,
            follow,
            tail,
            timestamps,
        } => {
            container_logs(&mut client, &container_id, follow, &tail, timestamps).await?;
        }
        ContainerCommands::Exec {
            container_id,
            tty,
            interactive,
            command,
        } => {
            container_exec(&mut client, &container_id, tty, interactive, command).await?;
        }
        ContainerCommands::Attach { container_id } => {
            container_attach(&mut client, &container_id).await?;
        }
        ContainerCommands::Wait { container_id } => {
            container_wait(&mut client, &container_id).await?;
        }
        ContainerCommands::Kill {
            container_id,
            signal,
        } => {
            container_kill(&mut client, &container_id, &signal).await?;
        }
        ContainerCommands::Rename {
            container_id,
            new_name,
        } => {
            container_rename(&mut client, &container_id, &new_name).await?;
        }
        ContainerCommands::Stats {
            container_id,
            no_stream,
        } => {
            container_stats(&mut client, &container_id, no_stream).await?;
        }
    }

    Ok(())
}

async fn container_create(
    client: &mut ContainerServiceClient<tonic::transport::Channel>,
    image: &str,
    name: Option<String>,
    env: Vec<String>,
    publish: Vec<String>,
    volume: Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let port_bindings = publish
        .iter()
        .filter_map(|p| {
            let parts: Vec<&str> = p.split(':').collect();
            if parts.len() == 2 {
                Some(PortBinding {
                    host_ip: String::new(),
                    host_port: parts[0].to_string(),
                    container_port: parts[1].to_string(),
                    protocol: "tcp".to_string(),
                })
            } else {
                eprintln!(
                    "Warning: Invalid port format '{}', expected HOST:CONTAINER",
                    p
                );
                None
            }
        })
        .collect();

    let binds = volume.iter().map(|v| v.to_string()).collect();

    let config = ContainerConfig {
        image: image.to_string(),
        env,
        ..Default::default()
    };

    let host_config = HostConfig {
        port_bindings,
        binds,
        ..Default::default()
    };

    let response = client
        .create_container(CreateContainerRequest {
            name: name.unwrap_or_default(),
            config: Some(config),
            host_config: Some(host_config),
            networking_config: None,
        })
        .await
        .map_err(|e| format!("Failed to create container: {}", e))?;

    let result = response.into_inner();
    println!("{}", result.id);

    if !result.warnings.is_empty() {
        for warning in &result.warnings {
            eprintln!("Warning: {}", warning);
        }
    }

    Ok(())
}

async fn container_start(
    client: &mut ContainerServiceClient<tonic::transport::Channel>,
    container_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    client
        .start_container(StartContainerRequest {
            container_id: container_id.to_string(),
            detach_keys: String::new(),
        })
        .await
        .map_err(|e| format!("Failed to start container: {}", e))?;

    println!("{}", container_id);
    Ok(())
}

async fn container_stop(
    client: &mut ContainerServiceClient<tonic::transport::Channel>,
    container_id: &str,
    timeout: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    client
        .stop_container(StopContainerRequest {
            container_id: container_id.to_string(),
            timeout,
        })
        .await
        .map_err(|e| format!("Failed to stop container: {}", e))?;

    println!("{}", container_id);
    Ok(())
}

async fn container_restart(
    client: &mut ContainerServiceClient<tonic::transport::Channel>,
    container_id: &str,
    timeout: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    client
        .restart_container(RestartContainerRequest {
            container_id: container_id.to_string(),
            timeout,
        })
        .await
        .map_err(|e| format!("Failed to restart container: {}", e))?;

    println!("{}", container_id);
    Ok(())
}

async fn container_list(
    client: &mut ContainerServiceClient<tonic::transport::Channel>,
    all: bool,
    limit: Option<i32>,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = client
        .list_containers(ListContainersRequest {
            all,
            limit: limit.unwrap_or(0),
            size: false,
            filters: Default::default(),
        })
        .await
        .map_err(|e| format!("Failed to list containers: {}", e))?;

    let containers = response.into_inner().containers;

    if containers.is_empty() {
        println!("No containers found");
        return Ok(());
    }

    println!(
        "{:<15} {:<20} {:<25} {:<20} {:<20}",
        "CONTAINER ID", "IMAGE", "COMMAND", "STATUS", "NAMES"
    );

    for container in containers {
        let id = if container.id.len() > 12 {
            &container.id[..12]
        } else {
            &container.id
        };

        let image = if container.image.len() > 18 {
            format!("{}...", &container.image[..15])
        } else {
            container.image.clone()
        };

        let command = if container.command.len() > 23 {
            format!("\"{}...\"", &container.command[..20])
        } else {
            format!("\"{}\"", container.command)
        };

        let names = container.names.join(", ");
        let names = if names.len() > 18 {
            format!("{}...", &names[..15])
        } else {
            names
        };

        println!(
            "{:<15} {:<20} {:<25} {:<20} {:<20}",
            id, image, command, container.status, names
        );
    }

    Ok(())
}

async fn container_inspect(
    client: &mut ContainerServiceClient<tonic::transport::Channel>,
    container_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = client
        .inspect_container(InspectContainerRequest {
            container_id: container_id.to_string(),
            size: false,
        })
        .await
        .map_err(|e| format!("Failed to inspect container: {}", e))?;

    let inspect = response.into_inner();

    println!("[{{");
    println!("    \"Id\": \"{}\",", container_id);
    println!("    \"Name\": \"{}\",", inspect.name);
    println!("    \"Path\": \"{}\",", inspect.path);
    println!("    \"Args\": {:?},", inspect.args);

    if let Some(state) = inspect.state {
        println!("    \"State\": {{");
        println!("        \"Status\": \"{}\",", state.status);
        println!("        \"Running\": {},", state.running);
        println!("        \"Paused\": {},", state.paused);
        println!("        \"Restarting\": {},", state.restarting);
        println!("        \"OOMKilled\": {},", state.oom_killed);
        println!("        \"Dead\": {},", state.dead);
        println!("        \"Pid\": {},", state.pid);
        println!("        \"ExitCode\": {},", state.exit_code);
        println!("        \"Error\": \"{}\"", state.error);
        println!("    }},");
    }

    if let Some(container) = inspect.container {
        println!("    \"Image\": \"{}\",", container.image);
        println!("    \"ImageID\": \"{}\",", container.image_id);

        if !container.labels.is_empty() {
            println!("    \"Labels\": {{");
            let labels: Vec<_> = container.labels.iter().collect();
            for (i, (key, value)) in labels.iter().enumerate() {
                let comma = if i < labels.len() - 1 { "," } else { "" };
                println!("        \"{}\": \"{}\"{}", key, value, comma);
            }
            println!("    }},");
        }
    }

    if let Some(config) = inspect.config {
        println!("    \"Config\": {{");
        println!("        \"Hostname\": \"{}\",", config.hostname);
        println!("        \"User\": \"{}\",", config.user);
        println!("        \"Env\": {:?},", config.env);
        println!("        \"Cmd\": {:?},", config.cmd);
        println!("        \"Image\": \"{}\",", config.image);
        println!("        \"WorkingDir\": \"{}\"", config.working_dir);
        println!("    }},");
    }

    println!("    \"Driver\": \"{}\",", inspect.driver);
    println!("    \"Platform\": \"{}\",", inspect.platform);
    println!("    \"RestartCount\": {}", inspect.restart_count);
    println!("}}]");

    Ok(())
}

async fn container_remove(
    client: &mut ContainerServiceClient<tonic::transport::Channel>,
    container_id: &str,
    force: bool,
    volumes: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    client
        .remove_container(RemoveContainerRequest {
            container_id: container_id.to_string(),
            force,
            remove_volumes: volumes,
            link: false,
        })
        .await
        .map_err(|e| format!("Failed to remove container: {}", e))?;

    println!("{}", container_id);
    Ok(())
}

async fn container_pause(
    client: &mut ContainerServiceClient<tonic::transport::Channel>,
    container_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    client
        .pause_container(PauseContainerRequest {
            container_id: container_id.to_string(),
        })
        .await
        .map_err(|e| format!("Failed to pause container: {}", e))?;

    println!("{}", container_id);
    Ok(())
}

async fn container_unpause(
    client: &mut ContainerServiceClient<tonic::transport::Channel>,
    container_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    client
        .unpause_container(UnpauseContainerRequest {
            container_id: container_id.to_string(),
        })
        .await
        .map_err(|e| format!("Failed to unpause container: {}", e))?;

    println!("{}", container_id);
    Ok(())
}

async fn container_logs(
    client: &mut ContainerServiceClient<tonic::transport::Channel>,
    container_id: &str,
    follow: bool,
    tail: &str,
    timestamps: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = client
        .get_logs(GetLogsRequest {
            container_id: container_id.to_string(),
            follow,
            stdout: true,
            stderr: true,
            since: None,
            until: None,
            timestamps,
            tail: tail.to_string(),
        })
        .await
        .map_err(|e| format!("Failed to get logs: {}", e))?
        .into_inner();

    while let Some(entry) = stream.next().await {
        match entry {
            Ok(log) => {
                if timestamps && let Some(ts) = log.timestamp {
                    print!("{}  ", format_timestamp(&ts));
                }
                print!("{}", log.message);
                if !log.message.ends_with('\n') {
                    println!();
                }
            }
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

async fn container_exec(
    client: &mut ContainerServiceClient<tonic::transport::Channel>,
    container_id: &str,
    tty: bool,
    interactive: bool,
    command: Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = ExecConfig {
        attach_stdin: interactive,
        attach_stdout: true,
        attach_stderr: true,
        detach_keys: String::new(),
        tty,
        env: vec![],
        cmd: command,
        privileged: false,
        user: String::new(),
        working_dir: String::new(),
    };

    let exec_response = client
        .exec(ExecRequest {
            container_id: container_id.to_string(),
            config: Some(config),
        })
        .await
        .map_err(|e| format!("Failed to create exec instance: {}", e))?;

    let exec_id = exec_response.into_inner().exec_id;

    let mut stream = client
        .exec_start(ExecStartRequest {
            exec_id,
            detach: false,
            tty,
        })
        .await
        .map_err(|e| format!("Failed to start exec: {}", e))?
        .into_inner();

    while let Some(output) = stream.next().await {
        match output {
            Ok(o) => {
                let data = String::from_utf8_lossy(&o.data);
                print!("{}", data);
            }
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

async fn container_attach(
    client: &mut ContainerServiceClient<tonic::transport::Channel>,
    container_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Attaching to container {}...", container_id);
    println!("(Press Ctrl+C to detach)");

    let request_stream = tokio_stream::iter(vec![AttachRequest {
        container_id: container_id.to_string(),
        stream: true,
        stdin: true,
        stdout: true,
        stderr: true,
        detach_keys: String::new(),
        logs: false,
        input: vec![],
    }]);

    let mut stream = client
        .attach(request_stream)
        .await
        .map_err(|e| format!("Failed to attach to container: {}", e))?
        .into_inner();

    while let Some(output) = stream.next().await {
        match output {
            Ok(o) => {
                let data = String::from_utf8_lossy(&o.data);
                print!("{}", data);
            }
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

async fn container_wait(
    client: &mut ContainerServiceClient<tonic::transport::Channel>,
    container_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = client
        .wait(WaitContainerRequest {
            container_id: container_id.to_string(),
            condition: String::new(),
        })
        .await
        .map_err(|e| format!("Failed to wait for container: {}", e))?;

    let result = response.into_inner();
    println!("{}", result.status_code);

    if let Some(err) = result.error
        && !err.message.is_empty()
    {
        eprintln!("Error: {}", err.message);
    }

    Ok(())
}

async fn container_kill(
    client: &mut ContainerServiceClient<tonic::transport::Channel>,
    container_id: &str,
    signal: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    client
        .kill(KillContainerRequest {
            container_id: container_id.to_string(),
            signal: signal.to_string(),
        })
        .await
        .map_err(|e| format!("Failed to kill container: {}", e))?;

    println!("{}", container_id);
    Ok(())
}

async fn container_rename(
    client: &mut ContainerServiceClient<tonic::transport::Channel>,
    container_id: &str,
    new_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    client
        .rename(RenameContainerRequest {
            container_id: container_id.to_string(),
            new_name: new_name.to_string(),
        })
        .await
        .map_err(|e| format!("Failed to rename container: {}", e))?;

    println!("{} -> {}", container_id, new_name);
    Ok(())
}

async fn container_stats(
    client: &mut ContainerServiceClient<tonic::transport::Channel>,
    container_id: &str,
    no_stream: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = client
        .stats(StatsRequest {
            container_id: container_id.to_string(),
            stream: !no_stream,
            one_shot: no_stream,
        })
        .await
        .map_err(|e| format!("Failed to get stats: {}", e))?
        .into_inner();

    println!(
        "{:<15} {:<10} {:<25} {:<15} {:<10}",
        "CONTAINER ID", "CPU %", "MEM USAGE / LIMIT", "MEM %", "PIDS"
    );

    let container_id_short = if container_id.len() > 12 {
        &container_id[..12]
    } else {
        container_id
    };

    while let Some(stats) = stream.next().await {
        match stats {
            Ok(s) => {
                let cpu_percent = calculate_cpu_percent(&s);
                let (mem_usage, mem_limit, mem_percent) = calculate_memory(&s);
                let pids = s.pids_stats.as_ref().map(|p| p.current).unwrap_or(0);

                println!(
                    "{:<15} {:<10.2} {:<25} {:<15.2} {:<10}",
                    container_id_short,
                    cpu_percent,
                    format!("{} / {}", format_size(mem_usage), format_size(mem_limit)),
                    mem_percent,
                    pids
                );

                if no_stream {
                    break;
                }
            }
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

fn calculate_cpu_percent(stats: &ross_core::ross::StatsResponse) -> f64 {
    let cpu_stats = match &stats.cpu_stats {
        Some(s) => s,
        None => return 0.0,
    };

    let precpu_stats = match &stats.precpu_stats {
        Some(s) => s,
        None => return 0.0,
    };

    let cpu_usage = cpu_stats
        .cpu_usage
        .as_ref()
        .map(|u| u.total_usage)
        .unwrap_or(0);
    let precpu_usage = precpu_stats
        .cpu_usage
        .as_ref()
        .map(|u| u.total_usage)
        .unwrap_or(0);

    let system_usage = cpu_stats.system_cpu_usage;
    let presystem_usage = precpu_stats.system_cpu_usage;

    let cpu_delta = cpu_usage.saturating_sub(precpu_usage) as f64;
    let system_delta = system_usage.saturating_sub(presystem_usage) as f64;

    if system_delta > 0.0 && cpu_delta > 0.0 {
        let online_cpus = cpu_stats.online_cpus.max(1) as f64;
        (cpu_delta / system_delta) * online_cpus * 100.0
    } else {
        0.0
    }
}

fn calculate_memory(stats: &ross_core::ross::StatsResponse) -> (u64, u64, f64) {
    let mem_stats = match &stats.memory_stats {
        Some(s) => s,
        None => return (0, 0, 0.0),
    };

    let usage = mem_stats.usage;
    let limit = mem_stats.limit;
    let percent = if limit > 0 {
        (usage as f64 / limit as f64) * 100.0
    } else {
        0.0
    };

    (usage, limit, percent)
}
