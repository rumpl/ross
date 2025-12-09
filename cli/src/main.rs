mod commands;
mod utils;

use clap::{Parser, Subcommand};
use commands::{
    handle_container_command, handle_image_command, health_check, ContainerCommands,
    ImageCommands,
};

#[derive(Parser)]
#[command(name = "ross")]
#[command(about = "Ross CLI - interact with the Ross daemon")]
struct Cli {
    /// Host address of the daemon
    #[arg(long, global = true, default_value = "127.0.0.1")]
    host: String,

    /// Port of the daemon
    #[arg(long, global = true, default_value_t = 50051)]
    port: u16,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Check the health of the daemon
    Health,
    /// Manage images
    #[command(subcommand)]
    Image(ImageCommands),
    /// Manage containers
    #[command(subcommand)]
    Container(ContainerCommands),
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let daemon_addr = format!("http://{}:{}", cli.host, cli.port);

    match cli.command {
        Some(Commands::Health) => {
            health_check(&daemon_addr).await?;
        }
        Some(Commands::Image(cmd)) => {
            handle_image_command(&daemon_addr, cmd).await?;
        }
        Some(Commands::Container(cmd)) => {
            handle_container_command(&daemon_addr, cmd).await?;
        }
        None => {
            println!("Ross CLI ready. Daemon address: {}:{}", cli.host, cli.port);
            println!("Use --help for usage information.");
        }
    }

    Ok(())
}
