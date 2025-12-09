mod services;

use clap::{Parser, Subcommand};
use ross_container::ContainerService;
use ross_core::container_service_server::ContainerServiceServer;
use ross_core::image_service_server::ImageServiceServer;
use ross_core::ross_server::RossServer;
use ross_core::snapshotter_service_server::SnapshotterServiceServer;
use ross_image::ImageService;
use ross_snapshotter::OverlaySnapshotter;
use ross_store::FileSystemStore;
use services::{ContainerServiceGrpc, ImageServiceGrpc, RossService, SnapshotterServiceGrpc};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::signal;
use tonic::transport::Server;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "ross-daemon")]
#[command(about = "Ross daemon gRPC server")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the gRPC server
    Start {
        /// Host address to bind to
        #[arg(long, default_value = "0.0.0.0")]
        host: String,

        /// Port to listen on
        #[arg(long, default_value_t = 50051)]
        port: u16,

        /// Data directory for storing images
        #[arg(long, default_value = "/tmp/ross")]
        data_dir: PathBuf,

        /// Maximum number of parallel blob downloads
        #[arg(long, default_value_t = 3)]
        max_concurrent_downloads: usize,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,ross=debug")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Start {
            host,
            port,
            data_dir,
            max_concurrent_downloads,
        } => {
            let addr = format!("{}:{}", host, port).parse()?;

            let store_path = data_dir.join("store");
            tracing::info!("Initializing store at {:?}", store_path);
            let store = FileSystemStore::new(&store_path).await?;
            let store = Arc::new(store);

            let snapshotter_path = data_dir.join("snapshotter");
            tracing::info!("Initializing snapshotter at {:?}", snapshotter_path);
            let snapshotter = OverlaySnapshotter::new(&snapshotter_path, store.clone()).await?;
            let snapshotter = Arc::new(snapshotter);

            tracing::info!("Initializing container service");
            let container_service =
                ContainerService::new(&data_dir, snapshotter.clone(), store.clone()).await?;
            let container_service = Arc::new(container_service);

            let image_service = Arc::new(ImageService::new(
                store.clone(),
                snapshotter.clone(),
                max_concurrent_downloads,
            ));

            tracing::info!(
                "Starting Ross daemon gRPC server on {} (max concurrent downloads: {})",
                addr,
                max_concurrent_downloads
            );

            Server::builder()
                .add_service(RossServer::new(RossService))
                .add_service(ImageServiceServer::new(ImageServiceGrpc::new(
                    image_service,
                )))
                .add_service(ContainerServiceServer::new(ContainerServiceGrpc::new(
                    container_service,
                )))
                .add_service(SnapshotterServiceServer::new(SnapshotterServiceGrpc::new(
                    snapshotter,
                )))
                .serve_with_shutdown(addr, async {
                    signal::ctrl_c()
                        .await
                        .expect("failed to listen for ctrl-c");
                    tracing::info!("Received shutdown signal, stopping server...");
                })
                .await?;
        }
    }

    Ok(())
}
