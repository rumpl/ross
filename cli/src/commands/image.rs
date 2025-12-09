use clap::Subcommand;
use ross_core::ross::image_service_client::ImageServiceClient;
use ross_core::ross::{
    BuildImageRequest, InspectImageRequest, ListImagesRequest, PullImageProgress, PullImageRequest,
    PushImageRequest, RemoveImageRequest, SearchImagesRequest, TagImageRequest,
};
use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};
use tokio_stream::StreamExt;

use crate::utils::format_size;

#[derive(Subcommand)]
pub enum ImageCommands {
    /// List images
    List {
        /// Show all images (default hides intermediate images)
        #[arg(long, short)]
        all: bool,

        /// Show digests
        #[arg(long)]
        digests: bool,
    },
    /// Display detailed information on one or more images
    Inspect {
        /// Image ID or name
        image_id: String,
    },
    /// Pull an image from a registry
    Pull {
        /// Image name
        image_name: String,

        /// Tag to pull
        #[arg(long, short, default_value = "latest")]
        tag: String,
    },
    /// Push an image to a registry
    Push {
        /// Image name
        image_name: String,

        /// Tag to push
        #[arg(long, short, default_value = "latest")]
        tag: String,
    },
    /// Build an image from a Dockerfile
    Build {
        /// Path to Dockerfile
        #[arg(long, default_value = "Dockerfile")]
        dockerfile: String,

        /// Name and optionally a tag in the name:tag format
        #[arg(long, short)]
        tag: Vec<String>,

        /// Do not use cache when building the image
        #[arg(long)]
        no_cache: bool,
    },
    /// Remove one or more images
    #[command(name = "remove", visible_alias = "rm")]
    Remove {
        /// Image ID or name
        image_id: String,

        /// Force removal of the image
        #[arg(long, short)]
        force: bool,

        /// Prune children images
        #[arg(long)]
        prune: bool,
    },
    /// Create a tag TARGET_IMAGE that refers to SOURCE_IMAGE
    Tag {
        /// Source image
        source_image: String,

        /// Target repository
        repository: String,

        /// Tag name
        #[arg(long, short, default_value = "latest")]
        tag: String,
    },
    /// Search the Docker Hub for images
    Search {
        /// Search term
        term: String,

        /// Maximum number of results
        #[arg(long, default_value_t = 25)]
        limit: i32,
    },
}

pub async fn handle_image_command(
    addr: &str,
    cmd: ImageCommands,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut client = ImageServiceClient::connect(addr.to_string())
        .await
        .map_err(|e| {
            format!(
                "Failed to connect to daemon at {}: {}. Is the daemon running?",
                addr, e
            )
        })?;

    match cmd {
        ImageCommands::List { all, digests } => {
            image_list(&mut client, all, digests).await?;
        }
        ImageCommands::Inspect { image_id } => {
            image_inspect(&mut client, &image_id).await?;
        }
        ImageCommands::Pull { image_name, tag } => {
            image_pull(&mut client, &image_name, &tag).await?;
        }
        ImageCommands::Push { image_name, tag } => {
            image_push(&mut client, &image_name, &tag).await?;
        }
        ImageCommands::Build {
            dockerfile,
            tag,
            no_cache,
        } => {
            image_build(&mut client, &dockerfile, tag, no_cache).await?;
        }
        ImageCommands::Remove {
            image_id,
            force,
            prune,
        } => {
            image_remove(&mut client, &image_id, force, prune).await?;
        }
        ImageCommands::Tag {
            source_image,
            repository,
            tag,
        } => {
            image_tag(&mut client, &source_image, &repository, &tag).await?;
        }
        ImageCommands::Search { term, limit } => {
            image_search(&mut client, &term, limit).await?;
        }
    }

    Ok(())
}

async fn image_list(
    client: &mut ImageServiceClient<tonic::transport::Channel>,
    all: bool,
    digests: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = client
        .list_images(ListImagesRequest {
            all,
            digests,
            filters: Default::default(),
        })
        .await
        .map_err(|e| format!("Failed to list images: {}", e))?;

    let images = response.into_inner().images;

    if images.is_empty() {
        println!("No images found");
        return Ok(());
    }

    if digests {
        println!(
            "{:<20} {:<15} {:<72} {:<15} {:<10}",
            "REPOSITORY", "TAG", "DIGEST", "IMAGE ID", "SIZE"
        );
    } else {
        println!(
            "{:<40} {:<15} {:<15} {:<10}",
            "REPOSITORY", "TAG", "IMAGE ID", "SIZE"
        );
    }

    for image in images {
        let id = image.id.trim_start_matches("sha256:");
        let id_short = if id.len() > 12 { &id[..12] } else { id };
        let size = format_size(image.size as u64);

        if image.repo_tags.is_empty() {
            if digests {
                let digest = image.repo_digests.first().map(|d| d.as_str()).unwrap_or("");
                println!(
                    "{:<20} {:<15} {:<72} {:<15} {:<10}",
                    "<none>", "<none>", digest, id_short, size
                );
            } else {
                println!(
                    "{:<40} {:<15} {:<15} {:<10}",
                    "<none>", "<none>", id_short, size
                );
            }
        } else {
            for repo_tag in &image.repo_tags {
                let parts: Vec<&str> = repo_tag.rsplitn(2, ':').collect();
                let (tag, repo) = if parts.len() == 2 {
                    (parts[0], parts[1])
                } else {
                    ("latest", repo_tag.as_str())
                };

                if digests {
                    let digest = image.repo_digests.first().map(|d| d.as_str()).unwrap_or("");
                    println!(
                        "{:<20} {:<15} {:<72} {:<15} {:<10}",
                        repo, tag, digest, id_short, size
                    );
                } else {
                    println!("{:<40} {:<15} {:<15} {:<10}", repo, tag, id_short, size);
                }
            }
        }
    }

    Ok(())
}

async fn image_inspect(
    client: &mut ImageServiceClient<tonic::transport::Channel>,
    image_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = client
        .inspect_image(InspectImageRequest {
            image_id: image_id.to_string(),
        })
        .await
        .map_err(|e| format!("Failed to inspect image: {}", e))?;

    let inspect = response.into_inner();

    if let Some(image) = inspect.image {
        println!("Image: {}", image_id);
        println!("  ID: {}", image.id);
        println!("  RepoTags: {:?}", image.repo_tags);
        println!("  RepoDigests: {:?}", image.repo_digests);
        println!("  Parent: {}", image.parent);
        println!("  Comment: {}", image.comment);
        println!("  Architecture: {}", image.architecture);
        println!("  OS: {}", image.os);
        println!("  Size: {}", format_size(image.size as u64));
        println!("  VirtualSize: {}", format_size(image.virtual_size as u64));
        println!("  Author: {}", image.author);
        println!("  DockerVersion: {}", image.docker_version);

        if !image.labels.is_empty() {
            println!("  Labels:");
            for (key, value) in &image.labels {
                println!("    {}: {}", key, value);
            }
        }

        if let Some(root_fs) = image.root_fs {
            println!("  RootFS:");
            println!("    Type: {}", root_fs.r#type);
            println!("    Layers: {} layer(s)", root_fs.layers.len());
        }
    } else {
        println!("Image not found: {}", image_id);
    }

    if !inspect.history.is_empty() {
        println!("\nHistory:");
        for (i, entry) in inspect.history.iter().enumerate() {
            println!("  [{}] {}", i, entry.created_by);
            if !entry.comment.is_empty() {
                println!("      Comment: {}", entry.comment);
            }
        }
    }

    Ok(())
}

struct PullProgressDisplay {
    layers: HashMap<String, LayerState>,
    layer_order: Vec<String>,
    footer_lines: Vec<String>,
    is_tty: bool,
    lines_printed: usize,
}

#[derive(Clone)]
struct LayerState {
    status: String,
    progress: String,
    done: bool,
}

impl PullProgressDisplay {
    fn new() -> Self {
        Self {
            layers: HashMap::new(),
            layer_order: Vec::new(),
            footer_lines: Vec::new(),
            is_tty: std::io::stdout().is_terminal(),
            lines_printed: 0,
        }
    }

    fn update(&mut self, progress: &PullImageProgress) {
        let id = &progress.id;

        if progress.status.starts_with("Resolving") || progress.status.starts_with("Resolved") {
            if self.is_tty {
                self.clear_lines();
                println!("{}: {}", id, progress.status);
                self.lines_printed = 0;
                self.redraw_all();
            } else {
                println!("{}: {}", id, progress.status);
            }
            return;
        }

        if progress.status.starts_with("Digest:") || progress.status.starts_with("Status:") {
            self.footer_lines
                .push(format!("{}: {}", id, progress.status));
            if self.is_tty {
                self.clear_lines();
                self.redraw_all();
            } else {
                println!("{}: {}", id, progress.status);
            }
            return;
        }

        if !self.layers.contains_key(id) {
            self.layer_order.push(id.clone());
            self.layers.insert(
                id.clone(),
                LayerState {
                    status: String::new(),
                    progress: String::new(),
                    done: false,
                },
            );
        }

        let done = progress.status == "Pull complete"
            || progress.status == "Already exists"
            || !progress.error.is_empty();

        let is_final_state =
            done || progress.status == "Downloading" || progress.status == "Pulling config";

        if let Some(state) = self.layers.get_mut(id) {
            state.status = if !progress.error.is_empty() {
                format!("Error: {}", progress.error)
            } else {
                progress.status.clone()
            };
            state.progress = progress.progress.clone();
            state.done = done;
        }

        if self.is_tty {
            self.clear_lines();
            self.redraw_all();
        } else if is_final_state && let Some(state) = self.layers.get(id) {
            println!("{}: {}", id, state.status);
        }
    }

    fn clear_lines(&self) {
        if self.lines_printed > 0 {
            print!("\x1b[{}A", self.lines_printed);
            for _ in 0..self.lines_printed {
                println!("\x1b[2K");
            }
            print!("\x1b[{}A", self.lines_printed);
        }
    }

    fn redraw_all(&mut self) {
        let mut stdout = io::stdout();
        let mut count = 0;

        for id in &self.layer_order {
            if let Some(state) = self.layers.get(id) {
                let status_icon = if state.done {
                    if state.status.starts_with("Error") {
                        "\x1b[31m✗\x1b[0m"
                    } else if state.status == "Already exists" {
                        "\x1b[33m≡\x1b[0m"
                    } else {
                        "\x1b[32m✓\x1b[0m"
                    }
                } else {
                    "\x1b[34m⠿\x1b[0m"
                };

                let line = if !state.progress.is_empty() {
                    format!(
                        "{} {}: {} {}",
                        status_icon, id, state.status, state.progress
                    )
                } else {
                    format!("{} {}: {}", status_icon, id, state.status)
                };

                println!("{}", line);
                count += 1;
            }
        }

        for line in &self.footer_lines {
            println!("{}", line);
            count += 1;
        }

        self.lines_printed = count;
        let _ = stdout.flush();
    }

    fn finish(&self) {
        if self.is_tty {
            let mut stdout = io::stdout();
            let _ = stdout.flush();
        }
    }
}

async fn image_pull(
    client: &mut ImageServiceClient<tonic::transport::Channel>,
    image_name: &str,
    tag: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Pulling {}:{}", image_name, tag);

    let mut stream = client
        .pull_image(PullImageRequest {
            image_name: image_name.to_string(),
            tag: tag.to_string(),
            registry_auth: None,
        })
        .await
        .map_err(|e| format!("Failed to pull image: {}", e))?
        .into_inner();

    let mut display = PullProgressDisplay::new();

    while let Some(progress) = stream.next().await {
        match progress {
            Ok(p) => {
                display.update(&p);
            }
            Err(e) => {
                eprintln!("\nStream error: {}", e);
                break;
            }
        }
    }

    display.finish();

    Ok(())
}

async fn image_push(
    client: &mut ImageServiceClient<tonic::transport::Channel>,
    image_name: &str,
    tag: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Pushing {}:{}", image_name, tag);

    let mut stream = client
        .push_image(PushImageRequest {
            image_name: image_name.to_string(),
            tag: tag.to_string(),
            registry_auth: None,
        })
        .await
        .map_err(|e| format!("Failed to push image: {}", e))?
        .into_inner();

    while let Some(progress) = stream.next().await {
        match progress {
            Ok(p) => {
                if !p.error.is_empty() {
                    eprintln!("Error: {}", p.error);
                } else if !p.progress.is_empty() {
                    println!("{}: {} {}", p.id, p.status, p.progress);
                } else {
                    println!("{}: {}", p.id, p.status);
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

async fn image_build(
    client: &mut ImageServiceClient<tonic::transport::Channel>,
    dockerfile: &str,
    tags: Vec<String>,
    no_cache: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Building image from {}", dockerfile);
    if !tags.is_empty() {
        println!("Tags: {}", tags.join(", "));
    }

    let mut stream = client
        .build_image(BuildImageRequest {
            dockerfile: dockerfile.to_string(),
            context_path: ".".to_string(),
            tags,
            build_args: Default::default(),
            no_cache,
            pull: false,
            target: String::new(),
            labels: Default::default(),
            platform: String::new(),
        })
        .await
        .map_err(|e| format!("Failed to build image: {}", e))?
        .into_inner();

    while let Some(progress) = stream.next().await {
        match progress {
            Ok(p) => {
                if !p.error.is_empty() {
                    eprintln!("Error: {}", p.error);
                } else if !p.stream.is_empty() {
                    print!("{}", p.stream);
                } else if !p.progress.is_empty() {
                    println!("{}", p.progress);
                }

                if let Some(aux) = p.aux
                    && !aux.id.is_empty()
                {
                    println!("Built image: {}", aux.id);
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

async fn image_remove(
    client: &mut ImageServiceClient<tonic::transport::Channel>,
    image_id: &str,
    force: bool,
    prune: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = client
        .remove_image(RemoveImageRequest {
            image_id: image_id.to_string(),
            force,
            prune_children: prune,
        })
        .await
        .map_err(|e| format!("Failed to remove image: {}", e))?;

    let result = response.into_inner();

    for untagged in &result.untagged {
        println!("Untagged: {}", untagged);
    }

    for deleted in &result.deleted {
        println!("Deleted: {}", deleted);
    }

    if result.deleted.is_empty() && result.untagged.is_empty() {
        println!("Image {} removed", image_id);
    }

    Ok(())
}

async fn image_tag(
    client: &mut ImageServiceClient<tonic::transport::Channel>,
    source_image: &str,
    repository: &str,
    tag: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = client
        .tag_image(TagImageRequest {
            source_image: source_image.to_string(),
            repository: repository.to_string(),
            tag: tag.to_string(),
        })
        .await
        .map_err(|e| format!("Failed to tag image: {}", e))?;

    let result = response.into_inner();

    if result.success {
        println!("Tagged {} as {}:{}", source_image, repository, tag);
    } else {
        eprintln!("Failed to tag image");
    }

    Ok(())
}

async fn image_search(
    client: &mut ImageServiceClient<tonic::transport::Channel>,
    term: &str,
    limit: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = client
        .search_images(SearchImagesRequest {
            term: term.to_string(),
            limit,
            filters: Default::default(),
        })
        .await
        .map_err(|e| format!("Failed to search images: {}", e))?;

    let results = response.into_inner().results;

    if results.is_empty() {
        println!("No results found for: {}", term);
        return Ok(());
    }

    println!(
        "{:<40} {:<60} {:<10} {:<10} {:<10}",
        "NAME", "DESCRIPTION", "STARS", "OFFICIAL", "AUTOMATED"
    );

    for result in results {
        let description = if result.description.len() > 57 {
            format!("{}...", &result.description[..57])
        } else {
            result.description.clone()
        };

        let official = if result.is_official { "[OK]" } else { "" };
        let automated = if result.is_automated { "[OK]" } else { "" };

        println!(
            "{:<40} {:<60} {:<10} {:<10} {:<10}",
            result.name, description, result.star_count, official, automated
        );
    }

    Ok(())
}
