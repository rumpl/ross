//! Host-side TTY handling for interactive containers with libkrun.
//!
//! This module runs on macOS and:
//! 1. Sets the terminal to raw mode
//! 2. Creates a Unix socket that libkrun maps to vsock
//! 3. Accepts the guest connection and forwards I/O

use crate::error::ShimError;
use crate::tty_protocol::*;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::net::UnixListener;

/// RAII guard for raw terminal mode.
/// Restores original terminal settings on drop.
pub struct RawTerminal {
    #[cfg(unix)]
    original: libc::termios,
}

impl RawTerminal {
    /// Set the terminal to raw mode and return a guard that restores it on drop.
    #[cfg(unix)]
    pub fn set() -> Result<RawTerminal, ShimError> {
        let fd = std::io::stdout().as_raw_fd();
        let mut original: libc::termios = unsafe { std::mem::zeroed() };

        if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
            return Err(ShimError::RuntimeError(
                "Failed to get terminal attributes".to_string(),
            ));
        }

        let mut raw = original;

        // Configure raw mode (similar to cfmakeraw)
        raw.c_iflag &= !(libc::IGNBRK
            | libc::BRKINT
            | libc::PARMRK
            | libc::ISTRIP
            | libc::INLCR
            | libc::IGNCR
            | libc::ICRNL
            | libc::IXON);
        raw.c_oflag &= !libc::OPOST;
        raw.c_lflag &= !(libc::ECHO | libc::ECHONL | libc::ICANON | libc::ISIG | libc::IEXTEN);
        raw.c_cflag &= !(libc::CSIZE | libc::PARENB);
        raw.c_cflag |= libc::CS8;
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;

        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return Err(ShimError::RuntimeError(
                "Failed to set raw mode".to_string(),
            ));
        }

        Ok(RawTerminal { original })
    }

    #[cfg(not(unix))]
    pub fn set() -> Result<RawTerminal, ShimError> {
        Err(ShimError::NotSupported(
            "Raw terminal not supported on this platform".to_string(),
        ))
    }
}

impl Drop for RawTerminal {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let fd = std::io::stdout().as_raw_fd();
            unsafe {
                libc::tcsetattr(fd, libc::TCSADRAIN, &self.original);
            }
        }
    }
}

/// Get current terminal size.
#[cfg(unix)]
pub fn get_terminal_size() -> Option<(u16, u16)> {
    let fd = std::io::stdout().as_raw_fd();
    unsafe {
        let mut size: libc::winsize = std::mem::zeroed();
        if libc::ioctl(fd, libc::TIOCGWINSZ, &mut size) == 0 {
            return Some((size.ws_col, size.ws_row));
        }
    }
    None
}

#[cfg(not(unix))]
pub fn get_terminal_size() -> Option<(u16, u16)> {
    None
}

/// Check if stdout is a TTY.
#[cfg(unix)]
pub fn is_tty() -> bool {
    unsafe { libc::isatty(std::io::stdout().as_raw_fd()) == 1 }
}

#[cfg(not(unix))]
pub fn is_tty() -> bool {
    false
}

/// Run the host-side I/O loop using poll() (macOS compatible).
#[cfg(unix)]
pub fn run_io_host(listener: UnixListener, is_tty: bool) -> Result<u8, ShimError> {
    use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
    use std::mem::ManuallyDrop;

    let mut stdin = ManuallyDrop::new(unsafe { File::from_raw_fd(0) });
    let mut stdout = ManuallyDrop::new(unsafe { File::from_raw_fd(1) });
    let mut stderr = ManuallyDrop::new(unsafe { File::from_raw_fd(2) });

    set_nonblocking(stdin.as_raw_fd())?;

    let (mut remote, _) = listener
        .accept()
        .map_err(|e| ShimError::RuntimeError(format!("Failed to accept connection: {}", e)))?;

    set_nonblocking(remote.as_raw_fd())?;

    if is_tty && let Some((cols, rows)) = get_terminal_size() {
        let _ = send_terminal_size(&mut remote, cols, rows);
    }

    #[cfg(target_os = "macos")]
    let _sigwinch_handler = if is_tty {
        Some(setup_sigwinch_handler()?)
    } else {
        None
    };

    loop {
        // Check for resize signal before polling
        #[cfg(target_os = "macos")]
        if is_tty
            && check_sigwinch_flag()
            && let Some((cols, rows)) = get_terminal_size()
        {
            let _ = send_terminal_size(&mut remote, cols, rows);
        }

        // Poll for events - store results as raw values to avoid borrow issues
        let (remote_ready, remote_hup, stdin_ready) = {
            let mut fds = [
                PollFd::new(remote.as_fd(), PollFlags::POLLIN),
                PollFd::new(stdin.as_fd(), PollFlags::POLLIN),
            ];

            match poll(&mut fds, PollTimeout::from(100u16)) {
                Ok(0) => continue,
                Ok(_) => {}
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => {
                    return Err(ShimError::RuntimeError(format!("poll failed: {}", e)));
                }
            }

            let remote_events = fds[0].revents();
            let stdin_events = fds[1].revents();

            let remote_ready = remote_events
                .map(|r| r.contains(PollFlags::POLLIN))
                .unwrap_or(false);
            let remote_hup = remote_events
                .map(|r| r.contains(PollFlags::POLLHUP) || r.contains(PollFlags::POLLERR))
                .unwrap_or(false);
            let stdin_ready = stdin_events
                .map(|r| r.contains(PollFlags::POLLIN))
                .unwrap_or(false);

            (remote_ready, remote_hup, stdin_ready)
        };

        // Process remote socket events
        if remote_ready {
            match process_guest_message(&mut remote, is_tty, &mut stdout, &mut stderr) {
                Ok(Some(exit_code)) => return Ok(exit_code),
                Ok(None) => {}
                Err(e) => {
                    tracing::debug!("Guest connection error: {}", e);
                    return Ok(1);
                }
            }
        }

        if remote_hup {
            tracing::debug!("Guest connection closed");
            return Ok(1);
        }

        // Process stdin events
        if stdin_ready {
            let mut buf = [0u8; 4096];
            match stdin.read(&mut buf) {
                Ok(0) => {
                    let cmd = encode_write_cmd(CMD_WRITE_STDIN, 0);
                    let _ = remote.write_all(&cmd.to_le_bytes());
                }
                Ok(n) => {
                    let cmd = encode_write_cmd(CMD_WRITE_STDIN, n);
                    if remote.write_all(&cmd.to_le_bytes()).is_err() {
                        return Ok(1);
                    }
                    if remote.write_all(&buf[..n]).is_err() {
                        return Ok(1);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => {}
            }
        }
    }
}

#[cfg(not(unix))]
pub fn run_io_host(_listener: UnixListener, _is_tty: bool) -> Result<u8, ShimError> {
    Err(ShimError::NotSupported(
        "run_io_host not supported on this platform".to_string(),
    ))
}

#[cfg(unix)]
fn set_nonblocking(fd: i32) -> Result<(), ShimError> {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 {
            return Err(ShimError::RuntimeError(
                "Failed to get fd flags".to_string(),
            ));
        }
        if libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(ShimError::RuntimeError(
                "Failed to set non-blocking".to_string(),
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn send_terminal_size(
    remote: &mut std::os::unix::net::UnixStream,
    cols: u16,
    rows: u16,
) -> std::io::Result<()> {
    let mut buf = [0u8; 6];
    buf[0..2].copy_from_slice(&CMD_UPDATE_SIZE.to_le_bytes());
    buf[2..4].copy_from_slice(&cols.to_le_bytes());
    buf[4..6].copy_from_slice(&rows.to_le_bytes());
    remote.write_all(&buf)
}

#[cfg(unix)]
fn process_guest_message(
    remote: &mut std::os::unix::net::UnixStream,
    is_tty: bool,
    stdout: &mut File,
    stderr: &mut File,
) -> Result<Option<u8>, ShimError> {
    let mut cmd_buf = [0u8; 2];
    match remote.read_exact(&mut cmd_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
        Err(e) => {
            return Err(ShimError::RuntimeError(format!(
                "Failed to read from guest: {}",
                e
            )))
        }
    }

    let cmd = u16::from_le_bytes(cmd_buf);
    let (opcode, value) = decode_cmd(cmd);

    match opcode {
        CMD_WRITE_STDOUT | CMD_WRITE_STDERR => {
            if value > 0 {
                let mut data = vec![0u8; value];
                match remote.read_exact(&mut data) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
                    Err(e) => {
                        return Err(ShimError::RuntimeError(format!(
                            "Failed to read data from guest: {}",
                            e
                        )))
                    }
                }

                let target = if opcode == CMD_WRITE_STDOUT || is_tty {
                    &mut *stdout
                } else {
                    &mut *stderr
                };

                target.write_all(&data).map_err(|e| {
                    ShimError::RuntimeError(format!("Failed to write to terminal: {}", e))
                })?;
                target.flush().ok();
            }
            Ok(None)
        }
        CMD_EXIT => Ok(Some(value as u8)),
        _ => {
            tracing::warn!("Unknown opcode from guest: {}", opcode);
            Ok(None)
        }
    }
}

#[cfg(target_os = "macos")]
mod sigwinch {
    use std::sync::atomic::{AtomicBool, Ordering};

    static SIGWINCH_RECEIVED: AtomicBool = AtomicBool::new(false);

    extern "C" fn sigwinch_handler(_: libc::c_int) {
        SIGWINCH_RECEIVED.store(true, Ordering::SeqCst);
    }

    pub fn setup() -> Result<(), super::ShimError> {
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = sigwinch_handler as usize;
            action.sa_flags = 0;

            if libc::sigaction(libc::SIGWINCH, &action, std::ptr::null_mut()) < 0 {
                return Err(super::ShimError::RuntimeError(
                    "Failed to set up SIGWINCH handler".to_string(),
                ));
            }
        }
        Ok(())
    }

    pub fn check_and_clear() -> bool {
        SIGWINCH_RECEIVED.swap(false, Ordering::SeqCst)
    }
}

#[cfg(target_os = "macos")]
fn setup_sigwinch_handler() -> Result<(), ShimError> {
    sigwinch::setup()
}

#[cfg(target_os = "macos")]
fn check_sigwinch_flag() -> bool {
    sigwinch::check_and_clear()
}

#[derive(Debug)]
pub enum HostIoEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Exit(u8),
}

/// Run the host-side I/O loop using channels for gRPC integration.
/// This version uses input_rx/output_tx channels instead of the daemon's terminal.
#[cfg(unix)]
pub fn run_io_host_with_channels(
    listener: UnixListener,
    is_tty: bool,
    input_rx: std::sync::mpsc::Receiver<crate::types::InputEvent>,
    output_tx: std::sync::mpsc::Sender<crate::types::OutputEvent>,
) -> Result<u8, ShimError> {
    use crate::types::{InputEvent, OutputEvent, WaitResult};
    use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

    let (mut remote, _) = listener
        .accept()
        .map_err(|e| ShimError::RuntimeError(format!("Failed to accept connection: {}", e)))?;

    set_nonblocking(remote.as_raw_fd())?;

    // Send initial terminal size if available
    if is_tty {
        if let Some((cols, rows)) = get_terminal_size() {
            let _ = send_terminal_size(&mut remote, cols, rows);
        }
    }

    loop {
        // Check for input from gRPC client (non-blocking)
        match input_rx.try_recv() {
            Ok(InputEvent::Stdin(data)) => {
                let cmd = encode_write_cmd(CMD_WRITE_STDIN, data.len());
                if remote.write_all(&cmd.to_le_bytes()).is_err() {
                    let _ = output_tx.send(OutputEvent::Exit(WaitResult {
                        exit_code: 1,
                        error: Some("Failed to write to guest".to_string()),
                    }));
                    return Ok(1);
                }
                if !data.is_empty() && remote.write_all(&data).is_err() {
                    let _ = output_tx.send(OutputEvent::Exit(WaitResult {
                        exit_code: 1,
                        error: Some("Failed to write to guest".to_string()),
                    }));
                    return Ok(1);
                }
            }
            Ok(InputEvent::Resize { width, height }) => {
                let _ = send_terminal_size(&mut remote, width, height);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // Client disconnected, send EOF to guest
                let cmd = encode_write_cmd(CMD_WRITE_STDIN, 0);
                let _ = remote.write_all(&cmd.to_le_bytes());
                return Ok(1);
            }
        }

        // Poll for events from guest
        let (remote_ready, remote_hup) = {
            let mut fds = [PollFd::new(remote.as_fd(), PollFlags::POLLIN)];

            match poll(&mut fds, PollTimeout::from(10u16)) {
                Ok(0) => continue,
                Ok(_) => {}
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => {
                    return Err(ShimError::RuntimeError(format!("poll failed: {}", e)));
                }
            }

            let remote_events = fds[0].revents();

            let remote_ready = remote_events
                .map(|r| r.contains(PollFlags::POLLIN))
                .unwrap_or(false);
            let remote_hup = remote_events
                .map(|r| r.contains(PollFlags::POLLHUP) || r.contains(PollFlags::POLLERR))
                .unwrap_or(false);

            (remote_ready, remote_hup)
        };

        // Process messages from guest
        if remote_ready {
            match process_guest_message_to_channel(&mut remote, is_tty, &output_tx) {
                Ok(Some(exit_code)) => {
                    let _ = output_tx.send(OutputEvent::Exit(WaitResult {
                        exit_code: exit_code as i32,
                        error: None,
                    }));
                    return Ok(exit_code);
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::debug!("Guest connection error: {}", e);
                    let _ = output_tx.send(OutputEvent::Exit(WaitResult {
                        exit_code: 1,
                        error: Some(e.to_string()),
                    }));
                    return Ok(1);
                }
            }
        }

        if remote_hup {
            tracing::debug!("Guest connection closed");
            let _ = output_tx.send(OutputEvent::Exit(WaitResult {
                exit_code: 1,
                error: None,
            }));
            return Ok(1);
        }
    }
}

#[cfg(unix)]
fn process_guest_message_to_channel(
    remote: &mut std::os::unix::net::UnixStream,
    is_tty: bool,
    output_tx: &std::sync::mpsc::Sender<crate::types::OutputEvent>,
) -> Result<Option<u8>, ShimError> {
    use crate::types::OutputEvent;

    let mut cmd_buf = [0u8; 2];
    match remote.read_exact(&mut cmd_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
        Err(e) => {
            return Err(ShimError::RuntimeError(format!(
                "Failed to read from guest: {}",
                e
            )))
        }
    }

    let cmd = u16::from_le_bytes(cmd_buf);
    let (opcode, value) = decode_cmd(cmd);

    match opcode {
        CMD_WRITE_STDOUT | CMD_WRITE_STDERR => {
            if value > 0 {
                let mut data = vec![0u8; value];
                match remote.read_exact(&mut data) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
                    Err(e) => {
                        return Err(ShimError::RuntimeError(format!(
                            "Failed to read data from guest: {}",
                            e
                        )))
                    }
                }

                let event = if opcode == CMD_WRITE_STDOUT || is_tty {
                    OutputEvent::Stdout(data)
                } else {
                    OutputEvent::Stderr(data)
                };

                output_tx.send(event).map_err(|e| {
                    ShimError::RuntimeError(format!("Failed to send output event: {}", e))
                })?;
            }
            Ok(None)
        }
        CMD_EXIT => Ok(Some(value as u8)),
        _ => {
            tracing::warn!("Unknown opcode from guest: {}", opcode);
            Ok(None)
        }
    }
}
