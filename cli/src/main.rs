mod commands;
mod utils;

use clap::{Parser, Subcommand};
use commands::{
    ContainerCommands, ImageCommands, handle_container_command, handle_image_command, health_check,
    run_container,
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
    /// Run a container (shorthand for container create + start)
    Run {
        /// Image to run
        image: String,

        /// Assign a name to the container
        #[arg(long)]
        name: Option<String>,

        /// Remove container when it exits
        #[arg(long)]
        rm: bool,

        /// Run container in the background
        #[arg(long, short)]
        detach: bool,

        /// Allocate a pseudo-TTY
        #[arg(long, short)]
        tty: bool,

        /// Keep STDIN open even if not attached
        #[arg(long, short)]
        interactive: bool,

        /// Set environment variables (KEY=VAL)
        #[arg(long, short)]
        env: Vec<String>,

        /// Publish a container's port(s) to the host (HOST:CONTAINER)
        #[arg(long = "publish", short = 'p')]
        publish: Vec<String>,

        /// Bind mount a volume (SRC:DST)
        #[arg(long, short)]
        volume: Vec<String>,

        /// Use host network
        #[arg(long)]
        network_host: bool,

        /// Command to run
        #[arg(last = true)]
        command: Vec<String>,
    },
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
        Some(Commands::Run {
            image,
            name,
            rm,
            detach,
            tty,
            interactive,
            env,
            publish,
            volume,
            network_host,
            command,
        }) => {
            run_container(
                &daemon_addr,
                &image,
                name,
                rm,
                detach,
                tty,
                interactive,
                env,
                publish,
                volume,
                network_host,
                command,
            )
            .await?;
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
