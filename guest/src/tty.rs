//! Guest-side TTY handling for interactive containers.
//!
//! This module runs inside the Linux VM and:
//! 1. Connects to the host via vsock
//! 2. Spawns the requested command with a PTY
//! 3. Forwards I/O between vsock and PTY
//!
//! NOTE: This module is Linux-only and must be cross-compiled for the guest VM.

use crate::protocol::*;
use crate::GuestConfig;
use std::ffi::CString;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};

fn set_window_size(fd: RawFd, cols: u16, rows: u16) -> std::io::Result<()> {
    #[repr(C)]
    struct Winsize {
        ws_row: u16,
        ws_col: u16,
        ws_xpixel: u16,
        ws_ypixel: u16,
    }

    let size = Winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    // TIOCSWINSZ = 0x5414 on Linux
    const TIOCSWINSZ: libc::c_int = 0x5414;

    if unsafe { libc::ioctl(fd, TIOCSWINSZ as _, &size) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn connect_vsock(port: u32) -> std::io::Result<RawFd> {
    // AF_VSOCK = 40 on Linux
    const AF_VSOCK: libc::c_int = 40;
    // VMADDR_CID_HOST = 2
    const VMADDR_CID_HOST: u32 = 2;

    #[repr(C)]
    struct SockaddrVm {
        svm_family: u16,
        svm_reserved1: u16,
        svm_port: u32,
        svm_cid: u32,
        svm_flags: u8,
        svm_zero: [u8; 3],
    }

    unsafe {
        let fd = libc::socket(AF_VSOCK, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        let addr = SockaddrVm {
            svm_family: AF_VSOCK as u16,
            svm_reserved1: 0,
            svm_port: port,
            svm_cid: VMADDR_CID_HOST,
            svm_flags: 0,
            svm_zero: [0; 3],
        };

        if libc::connect(
            fd,
            &addr as *const _ as *const libc::sockaddr,
            std::mem::size_of::<SockaddrVm>() as u32,
        ) < 0
        {
            libc::close(fd);
            return Err(std::io::Error::last_os_error());
        }

        Ok(fd)
    }
}

fn openpty() -> std::io::Result<(RawFd, RawFd)> {
    let mut master: libc::c_int = 0;
    let mut slave: libc::c_int = 0;

    if unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } < 0
    {
        return Err(std::io::Error::last_os_error());
    }

    Ok((master, slave))
}

fn run_io_loop_tty(
    pty_master: &mut File,
    vsock: &mut File,
    child_pid: libc::pid_t,
) -> std::io::Result<i32> {
    // Use poll instead of epoll for simpler code
    let pty_fd = pty_master.as_raw_fd();
    let vsock_fd = vsock.as_raw_fd();

    // Set non-blocking
    unsafe {
        let flags = libc::fcntl(pty_fd, libc::F_GETFL);
        libc::fcntl(pty_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        let flags = libc::fcntl(vsock_fd, libc::F_GETFL);
        libc::fcntl(vsock_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }

    let mut exit_code: Option<i32> = None;

    loop {
        // Check if child has exited
        let mut status: libc::c_int = 0;
        let wait_result = unsafe { libc::waitpid(child_pid, &mut status, libc::WNOHANG) };
        if wait_result > 0 {
            if libc::WIFEXITED(status) {
                exit_code = Some(libc::WEXITSTATUS(status));
            } else if libc::WIFSIGNALED(status) {
                exit_code = Some(128 + libc::WTERMSIG(status));
            }
        }

        let mut fds = [
            libc::pollfd {
                fd: pty_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: vsock_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];

        let poll_result = unsafe { libc::poll(fds.as_mut_ptr(), 2, 100) };

        if poll_result < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }

        // Check PTY for output from command
        if fds[0].revents & libc::POLLIN != 0 {
            let mut buf = [0u8; 4096];
            match pty_master.read(&mut buf) {
                Ok(0) => {
                    // PTY closed
                    if let Some(code) = exit_code {
                        let cmd = encode_exit_cmd(code as u8);
                        let _ = vsock.write_all(&cmd.to_le_bytes());
                        return Ok(code);
                    }
                }
                Ok(n) => {
                    let cmd = encode_write_cmd(CMD_WRITE_STDOUT, n);
                    vsock.write_all(&cmd.to_le_bytes())?;
                    vsock.write_all(&buf[..n])?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => {
                    if let Some(code) = exit_code {
                        let cmd = encode_exit_cmd(code as u8);
                        let _ = vsock.write_all(&cmd.to_le_bytes());
                        return Ok(code);
                    }
                }
            }
        }

        if fds[0].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            // PTY closed
            let code = exit_code.unwrap_or(0);
            let cmd = encode_exit_cmd(code as u8);
            let _ = vsock.write_all(&cmd.to_le_bytes());
            return Ok(code);
        }

        // Check vsock for input from host
        if fds[1].revents & libc::POLLIN != 0 {
            let mut cmd_buf = [0u8; 2];
            if vsock.read_exact(&mut cmd_buf).is_err() {
                let code = exit_code.unwrap_or(1);
                return Ok(code);
            }

            let cmd = u16::from_le_bytes(cmd_buf);
            let (opcode, value) = decode_cmd(cmd);

            match opcode {
                CMD_WRITE_STDIN => {
                    if value > 0 {
                        let mut data = vec![0u8; value];
                        if vsock.read_exact(&mut data).is_ok() {
                            let _ = pty_master.write_all(&data);
                        }
                    }
                }
                CMD_UPDATE_SIZE => {
                    let mut size_buf = [0u8; 4];
                    if vsock.read_exact(&mut size_buf).is_ok() {
                        let cols = u16::from_le_bytes([size_buf[0], size_buf[1]]);
                        let rows = u16::from_le_bytes([size_buf[2], size_buf[3]]);
                        let _ = set_window_size(pty_fd, cols, rows);
                    }
                }
                _ => {}
            }
        }

        if fds[1].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            // Vsock closed
            let code = exit_code.unwrap_or(1);
            return Ok(code);
        }

        // If child exited and PTY is drained, exit
        if exit_code.is_some() && poll_result == 0 {
            let code = exit_code.unwrap();
            let cmd = encode_exit_cmd(code as u8);
            let _ = vsock.write_all(&cmd.to_le_bytes());
            return Ok(code);
        }
    }
}

fn run_io_loop_pipes(
    stdin_pipe: &mut Option<File>,
    stdout_pipe: &mut File,
    stderr_pipe: &mut File,
    vsock: &mut File,
    child_pid: libc::pid_t,
) -> std::io::Result<i32> {
    let stdout_fd = stdout_pipe.as_raw_fd();
    let stderr_fd = stderr_pipe.as_raw_fd();
    let vsock_fd = vsock.as_raw_fd();

    // Set non-blocking
    unsafe {
        let flags = libc::fcntl(stdout_fd, libc::F_GETFL);
        libc::fcntl(stdout_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        let flags = libc::fcntl(stderr_fd, libc::F_GETFL);
        libc::fcntl(stderr_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        let flags = libc::fcntl(vsock_fd, libc::F_GETFL);
        libc::fcntl(vsock_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }

    let mut exit_code: Option<i32> = None;
    let mut stdout_closed = false;
    let mut stderr_closed = false;

    loop {
        // Check if child has exited
        let mut status: libc::c_int = 0;
        let wait_result = unsafe { libc::waitpid(child_pid, &mut status, libc::WNOHANG) };
        if wait_result > 0 {
            if libc::WIFEXITED(status) {
                exit_code = Some(libc::WEXITSTATUS(status));
            } else if libc::WIFSIGNALED(status) {
                exit_code = Some(128 + libc::WTERMSIG(status));
            }
        }

        let mut fds = [
            libc::pollfd {
                fd: stdout_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: stderr_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: vsock_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];

        let poll_result = unsafe { libc::poll(fds.as_mut_ptr(), 3, 100) };

        if poll_result < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }

        // Check stdout
        if fds[0].revents & libc::POLLIN != 0 {
            let mut buf = [0u8; 4096];
            match stdout_pipe.read(&mut buf) {
                Ok(0) => stdout_closed = true,
                Ok(n) => {
                    let cmd = encode_write_cmd(CMD_WRITE_STDOUT, n);
                    vsock.write_all(&cmd.to_le_bytes())?;
                    vsock.write_all(&buf[..n])?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => stdout_closed = true,
            }
        }
        if fds[0].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            stdout_closed = true;
        }

        // Check stderr
        if fds[1].revents & libc::POLLIN != 0 {
            let mut buf = [0u8; 4096];
            match stderr_pipe.read(&mut buf) {
                Ok(0) => stderr_closed = true,
                Ok(n) => {
                    let cmd = encode_write_cmd(CMD_WRITE_STDERR, n);
                    vsock.write_all(&cmd.to_le_bytes())?;
                    vsock.write_all(&buf[..n])?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => stderr_closed = true,
            }
        }
        if fds[1].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            stderr_closed = true;
        }

        // Check vsock for input from host
        if fds[2].revents & libc::POLLIN != 0 {
            let mut cmd_buf = [0u8; 2];
            if vsock.read_exact(&mut cmd_buf).is_err() {
                let code = exit_code.unwrap_or(1);
                return Ok(code);
            }

            let cmd = u16::from_le_bytes(cmd_buf);
            let (opcode, value) = decode_cmd(cmd);

            if opcode == CMD_WRITE_STDIN {
                if value == 0 {
                    *stdin_pipe = None;
                } else if let Some(stdin) = stdin_pipe {
                    let mut data = vec![0u8; value];
                    if vsock.read_exact(&mut data).is_ok() {
                        let _ = stdin.write_all(&data);
                    }
                }
            }
        }

        if fds[2].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            let code = exit_code.unwrap_or(1);
            return Ok(code);
        }

        // If child exited and pipes are drained, exit
        if exit_code.is_some() && stdout_closed && stderr_closed {
            let code = exit_code.unwrap();
            let cmd = encode_exit_cmd(code as u8);
            let _ = vsock.write_all(&cmd.to_le_bytes());
            return Ok(code);
        }
    }
}

/// Run a command and forward I/O via vsock.
///
/// This is the main entry point for the guest init process.
pub fn run_guest_command(config: &GuestConfig) -> std::io::Result<i32> {
    let vsock_fd = connect_vsock(config.vsock_port)?;
    let mut vsock = unsafe { File::from_raw_fd(vsock_fd) };

    if config.tty {
        let (master, slave) = openpty()?;

        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Err(std::io::Error::last_os_error());
        }

        if pid == 0 {
            // Child process
            unsafe {
                libc::close(master);
                libc::setsid();

                // TIOCSCTTY = 0x540E on Linux
                const TIOCSCTTY: libc::c_int = 0x540E;
                libc::ioctl(slave, TIOCSCTTY as _, 0);

                libc::dup2(slave, 0);
                libc::dup2(slave, 1);
                libc::dup2(slave, 2);

                if slave > 2 {
                    libc::close(slave);
                }
            }

            drop(vsock);

            if let Some(ref wd) = config.workdir {
                let _ = std::env::set_current_dir(wd);
            }

            for env_var in &config.env {
                if let Some((key, value)) = env_var.split_once('=') {
                    // SAFETY: We're in the child process after fork, before exec.
                    // No other threads exist, so modifying environment is safe.
                    unsafe { std::env::set_var(key, value) };
                }
            }

            let cmd = CString::new(config.command.as_str()).unwrap();
            let mut args: Vec<CString> = config
                .args
                .iter()
                .map(|s| CString::new(s.as_str()).unwrap())
                .collect();
            args.insert(0, cmd.clone());
            let arg_ptrs: Vec<*const libc::c_char> =
                args.iter().map(|s| s.as_ptr()).chain(std::iter::once(std::ptr::null())).collect();

            unsafe {
                libc::execvp(cmd.as_ptr(), arg_ptrs.as_ptr());
            }
            std::process::exit(127);
        }

        // Parent process
        unsafe { libc::close(slave) };
        let mut pty_master = unsafe { File::from_raw_fd(master) };

        run_io_loop_tty(&mut pty_master, &mut vsock, pid)
    } else {
        // Non-TTY mode: use pipes
        let mut stdin_pipe = [0i32; 2];
        let mut stdout_pipe = [0i32; 2];
        let mut stderr_pipe = [0i32; 2];

        unsafe {
            libc::pipe(stdin_pipe.as_mut_ptr());
            libc::pipe(stdout_pipe.as_mut_ptr());
            libc::pipe(stderr_pipe.as_mut_ptr());
        }

        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Err(std::io::Error::last_os_error());
        }

        if pid == 0 {
            // Child process
            unsafe {
                libc::close(stdin_pipe[1]);
                libc::close(stdout_pipe[0]);
                libc::close(stderr_pipe[0]);

                libc::dup2(stdin_pipe[0], 0);
                libc::dup2(stdout_pipe[1], 1);
                libc::dup2(stderr_pipe[1], 2);

                libc::close(stdin_pipe[0]);
                libc::close(stdout_pipe[1]);
                libc::close(stderr_pipe[1]);
            }

            drop(vsock);

            if let Some(ref wd) = config.workdir {
                let _ = std::env::set_current_dir(wd);
            }

            for env_var in &config.env {
                if let Some((key, value)) = env_var.split_once('=') {
                    // SAFETY: We're in the child process after fork, before exec.
                    // No other threads exist, so modifying environment is safe.
                    unsafe { std::env::set_var(key, value) };
                }
            }

            let cmd = CString::new(config.command.as_str()).unwrap();
            let mut args: Vec<CString> = config
                .args
                .iter()
                .map(|s| CString::new(s.as_str()).unwrap())
                .collect();
            args.insert(0, cmd.clone());
            let arg_ptrs: Vec<*const libc::c_char> =
                args.iter().map(|s| s.as_ptr()).chain(std::iter::once(std::ptr::null())).collect();

            unsafe {
                libc::execvp(cmd.as_ptr(), arg_ptrs.as_ptr());
            }
            std::process::exit(127);
        }

        // Parent process
        unsafe {
            libc::close(stdin_pipe[0]);
            libc::close(stdout_pipe[1]);
            libc::close(stderr_pipe[1]);
        }

        let mut stdin_file = Some(unsafe { File::from_raw_fd(stdin_pipe[1]) });
        let mut stdout_file = unsafe { File::from_raw_fd(stdout_pipe[0]) };
        let mut stderr_file = unsafe { File::from_raw_fd(stderr_pipe[0]) };

        run_io_loop_pipes(
            &mut stdin_file,
            &mut stdout_file,
            &mut stderr_file,
            &mut vsock,
            pid,
        )
    }
}
