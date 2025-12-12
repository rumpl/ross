//! Ross init - guest init process for interactive containers.
//!
//! This binary runs inside the VM as the init process. It reads a JSON
//! configuration specifying the command to run, then handles PTY/pipe
//! setup and I/O forwarding to the host via vsock.
//!
//! Usage:
//!   ross-init '<json-config>'
//!   ROSS_GUEST_CONFIG='<json-config>' ross-init

use ross_guest::{tty, GuestConfig};
use std::env;
use std::process::ExitCode;

const CONFIG_FILE_PATH: &str = "/.ross-config.json";

fn main() -> ExitCode {
    eprintln!("ross-init: starting");
    eprintln!("ross-init: args = {:?}", env::args().collect::<Vec<_>>());
    
    // Check if config file exists
    match std::fs::metadata(CONFIG_FILE_PATH) {
        Ok(m) => eprintln!("ross-init: config file exists, size = {}", m.len()),
        Err(e) => eprintln!("ross-init: config file check: {}", e),
    }
    
    // Try to read it
    match std::fs::read_to_string(CONFIG_FILE_PATH) {
        Ok(s) => eprintln!("ross-init: config file contents ({} bytes): {:?}", s.len(), &s[..s.len().min(100)]),
        Err(e) => eprintln!("ross-init: failed to read config file: {}", e),
    }
    
    // Check env var
    match env::var("ROSS_GUEST_CONFIG") {
        Ok(s) => eprintln!("ross-init: env var set, len = {}", s.len()),
        Err(_) => eprintln!("ross-init: env var not set"),
    }

    // Read config from: 1) command line (if it looks like JSON), 2) env var, 3) config file
    // Skip argv[1] if it looks like a path (starts with /)
    let config_json = env::args()
        .nth(1)
        .filter(|arg| arg.starts_with('{'))
        .or_else(|| env::var("ROSS_GUEST_CONFIG").ok())
        .or_else(|| std::fs::read_to_string(CONFIG_FILE_PATH).ok());

    let config_json = match config_json {
        Some(json) => json,
        None => {
            eprintln!("ross-init: no configuration provided");
            eprintln!("Usage: ross-init '<json-config>'");
            eprintln!("   or: ROSS_GUEST_CONFIG='<json>' ross-init");
            eprintln!("   or: place config at {}", CONFIG_FILE_PATH);
            return ExitCode::from(1);
        }
    };

    let config: GuestConfig = match serde_json::from_str(&config_json) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ross-init: failed to parse config: {}", e);
            eprintln!("ross-init: config_json len = {}, first 200 chars: {:?}", 
                config_json.len(), 
                &config_json[..config_json.len().min(200)]);
            return ExitCode::from(1);
        }
    };

    // Validate config
    if config.command.is_empty() {
        eprintln!("ross-init: command is empty");
        return ExitCode::from(1);
    }

    if config.vsock_port == 0 {
        eprintln!("ross-init: vsock_port is required for interactive mode");
        return ExitCode::from(1);
    }

    // Run the command
    match tty::run_guest_command(&config) {
        Ok(exit_code) => ExitCode::from(exit_code as u8),
        Err(e) => {
            eprintln!("ross-init: error running command: {}", e);
            ExitCode::from(1)
        }
    }
}
