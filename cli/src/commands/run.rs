use ross_core::ross::container_service_client::ContainerServiceClient;
use ross_core::ross::image_service_client::ImageServiceClient;
use ross_core::ross::{
    ContainerConfig, CreateContainerRequest, HostConfig, PortBinding, PullImageRequest,
    RemoveContainerRequest, StartContainerRequest, WaitContainerRequest,
    wait_container_output::Output,
};
use std::io::Write;
use tokio_stream::StreamExt;

#[allow(clippy::too_many_arguments)]
pub async fn run_container(
    addr: &str,
    image: &str,
    name: Option<String>,
    rm: bool,
    detach: bool,
    tty: bool,
    interactive: bool,
    env: Vec<String>,
    publish: Vec<String>,
    volume: Vec<String>,
    network_host: bool,
    command: Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut image_client = ImageServiceClient::connect(addr.to_string())
        .await
        .map_err(|e| {
            format!(
                "Failed to connect to daemon at {}: {}. Is the daemon running?",
                addr, e
            )
        })?;

    let mut container_client = ContainerServiceClient::connect(addr.to_string()).await?;

    let (image_name, tag) = parse_image_reference(image);

    eprintln!("Pulling image {}:{}...", image_name, tag);
    let mut pull_stream = image_client
        .pull_image(PullImageRequest {
            image_name: image_name.clone(),
            tag: tag.clone(),
            registry_auth: None,
        })
        .await
        .map_err(|e| format!("Failed to pull image: {}", e))?
        .into_inner();

    let mut image_id = String::new();
    while let Some(progress) = pull_stream.next().await {
        match progress {
            Ok(p) => {
                if !p.id.is_empty() {
                    image_id = p.id.clone();
                }
                if !p.status.is_empty() {
                    if !p.id.is_empty() {
                        eprintln!("{}: {}", p.id, p.status);
                    } else {
                        eprintln!("{}", p.status);
                    }
                }
            }
            Err(e) => {
                return Err(format!("Pull failed: {}", e).into());
            }
        }
    }

    if image_id.is_empty() {
        image_id = format!("{}:{}", image_name, tag);
    }

    eprintln!("Image pulled: {}", image_id);

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

    let config = ContainerConfig {
        image: image_id.clone(),
        env,
        cmd: command,
        tty,
        open_stdin: interactive,
        ..Default::default()
    };

    let network_mode = if network_host {
        "host".to_string()
    } else {
        String::new()
    };

    let host_config = HostConfig {
        port_bindings,
        binds: volume,
        auto_remove: rm,
        network_mode,
        ..Default::default()
    };

    eprintln!("Creating container...");
    let create_response = container_client
        .create_container(CreateContainerRequest {
            name: name.clone().unwrap_or_default(),
            config: Some(config),
            host_config: Some(host_config),
            networking_config: None,
        })
        .await
        .map_err(|e| format!("Failed to create container: {}", e))?;

    let container_id = create_response.into_inner().id;
    eprintln!("Container created: {}", container_id);

    if detach {
        // For detached mode, start the container and return immediately
        eprintln!("Starting container...");
        container_client
            .start_container(StartContainerRequest {
                container_id: container_id.clone(),
                detach_keys: String::new(),
            })
            .await
            .map_err(|e| format!("Failed to start container: {}", e))?;

        println!("{}", container_id);
        return Ok(());
    }

    // For attached mode, use wait which starts and streams output
    eprintln!("Starting and attaching to container...");
    let mut wait_stream = container_client
        .wait(WaitContainerRequest {
            container_id: container_id.clone(),
            condition: String::new(),
        })
        .await
        .map_err(|e| format!("Failed to start/wait for container: {}", e))?
        .into_inner();

    let mut exit_code: i64 = 0;
    
    while let Some(output) = wait_stream.next().await {
        match output {
            Ok(msg) => match msg.output {
                Some(Output::Data(data)) => {
                    if data.stream == "stdout" {
                        std::io::stdout().write_all(&data.data)?;
                        std::io::stdout().flush()?;
                    } else {
                        std::io::stderr().write_all(&data.data)?;
                        std::io::stderr().flush()?;
                    }
                }
                Some(Output::Exit(result)) => {
                    exit_code = result.status_code;
                    if let Some(err) = result.error {
                        eprintln!("Container error: {}", err.message);
                    }
                }
                None => {}
            },
            Err(e) => {
                eprintln!("Error reading container output: {}", e);
                break;
            }
        }
    }
    
    eprintln!("Container exited with code: {}", exit_code);

    if rm {
        eprintln!("Removing container...");
        container_client
            .remove_container(RemoveContainerRequest {
                container_id: container_id.clone(),
                force: false,
                remove_volumes: false,
                link: false,
            })
            .await
            .map_err(|e| format!("Failed to remove container: {}", e))?;
    }

    if exit_code != 0 {
        std::process::exit(exit_code as i32);
    }

    Ok(())
}

fn parse_image_reference(image: &str) -> (String, String) {
    if let Some(pos) = image.rfind(':') {
        let potential_tag = &image[pos + 1..];
        if !potential_tag.contains('/') {
            return (image[..pos].to_string(), potential_tag.to_string());
        }
    }
    (image.to_string(), "latest".to_string())
}
