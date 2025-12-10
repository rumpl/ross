use crate::error::ShimError;
use crate::rootfs;
use crate::shim::{OutputEventStream, Shim};
use crate::types::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(all(feature = "libkrun", target_os = "macos"))]
use std::os::unix::io::FromRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContainerMetadata {
    info: ContainerInfo,
    config: ContainerConfig,
    host_config: HostConfig,
}

pub struct KrunShim {
    data_dir: PathBuf,
    containers: Arc<RwLock<HashMap<String, ContainerMetadata>>>,
}

impl KrunShim {
    pub async fn new(data_dir: &Path) -> Result<Self, ShimError> {
        let containers_dir = data_dir.join("containers");
        fs::create_dir_all(&containers_dir).await?;

        let shim = Self {
            data_dir: data_dir.to_path_buf(),
            containers: Arc::new(RwLock::new(HashMap::new())),
        };

        shim.load_containers().await?;

        Ok(shim)
    }

    async fn load_containers(&self) -> Result<(), ShimError> {
        let containers_dir = self.data_dir.join("containers");
        let mut entries = fs::read_dir(&containers_dir).await?;
        let mut containers = self.containers.write().await;

        while let Some(entry) = entries.next_entry().await? {
            let metadata_path = entry.path().join("metadata.json");
            if metadata_path.exists() {
                let content = fs::read_to_string(&metadata_path).await?;
                if let Ok(metadata) = serde_json::from_str::<ContainerMetadata>(&content) {
                    containers.insert(metadata.info.id.clone(), metadata);
                }
            }
        }

        Ok(())
    }

    async fn save_container(&self, metadata: &ContainerMetadata) -> Result<(), ShimError> {
        let container_dir = self.data_dir.join("containers").join(&metadata.info.id);
        fs::create_dir_all(&container_dir).await?;
        let metadata_path = container_dir.join("metadata.json");
        let content = serde_json::to_string_pretty(metadata)?;
        fs::write(&metadata_path, content).await?;
        Ok(())
    }

    fn current_timestamp() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    /// Prepare the rootfs from overlay mount specifications.
    /// For libkrun, we need to copy all layers into a single directory.
    async fn prepare_rootfs_from_mounts(
        &self,
        mounts: &[SnapshotMount],
        target: &Path,
    ) -> Result<(), ShimError> {
        fs::create_dir_all(target).await?;

        for mount in mounts {
            match mount.mount_type.as_str() {
                "overlay" => {
                    // Parse overlay options to extract lowerdir and upperdir
                    let (lowerdirs, upperdir) = Self::parse_overlay_options(&mount.options)?;

                    // Copy lower dirs in reverse order (bottom to top)
                    for dir in lowerdirs.iter().rev() {
                        tracing::debug!("Copying lower layer: {}", dir);
                        Self::copy_dir_contents(Path::new(dir), target).await?;
                    }

                    // Copy upper dir last (it takes precedence)
                    if let Some(upper) = upperdir {
                        tracing::debug!("Copying upper layer: {}", upper);
                        Self::copy_dir_contents(Path::new(&upper), target).await?;
                    }
                }
                "bind" => {
                    // For bind mounts, just copy the source
                    tracing::debug!("Copying bind mount source: {}", mount.source);
                    Self::copy_dir_contents(Path::new(&mount.source), target).await?;
                }
                _ => {
                    tracing::warn!("Unknown mount type: {}", mount.mount_type);
                }
            }
        }

        // Ensure essential directories exist
        rootfs::ensure_essential_dirs(target).await?;

        Ok(())
    }

    fn parse_overlay_options(options: &[String]) -> Result<(Vec<String>, Option<String>), ShimError> {
        let mut lowerdirs = Vec::new();
        let mut upperdir = None;

        for opt in options {
            if let Some(dirs) = opt.strip_prefix("lowerdir=") {
                lowerdirs = dirs.split(':').map(String::from).collect();
            } else if let Some(dir) = opt.strip_prefix("upperdir=") {
                upperdir = Some(dir.to_string());
            }
        }

        Ok((lowerdirs, upperdir))
    }

    async fn copy_dir_contents(src: &Path, dst: &Path) -> Result<(), ShimError> {
        if !src.exists() {
            return Ok(());
        }

        let mut stack = vec![(src.to_path_buf(), PathBuf::new())];

        while let Some((current_src, relative)) = stack.pop() {
            let current_dst = dst.join(&relative);

            let mut entries = match fs::read_dir(&current_src).await {
                Ok(e) => e,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };

            while let Some(entry) = entries.next_entry().await? {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();

                // Skip whiteout files - they indicate deleted files
                if name_str.starts_with(".wh.") {
                    // Handle whiteout: delete the target file if it exists
                    if name_str == ".wh..wh..opq" {
                        // Opaque whiteout - clear the directory
                        if current_dst.exists() {
                            Self::clear_directory(&current_dst).await?;
                        }
                    } else {
                        // Regular whiteout
                        let target_name = name_str.strip_prefix(".wh.").unwrap();
                        let target_path = current_dst.join(target_name);
                        if target_path.exists() {
                            if target_path.is_dir() {
                                fs::remove_dir_all(&target_path).await?;
                            } else {
                                fs::remove_file(&target_path).await?;
                            }
                        }
                    }
                    continue;
                }

                let src_path = entry.path();
                let dst_path = current_dst.join(&name);
                let file_type = entry.file_type().await?;

                if file_type.is_dir() {
                    fs::create_dir_all(&dst_path).await?;
                    stack.push((src_path, relative.join(&name)));
                } else if file_type.is_file() {
                    if let Some(parent) = dst_path.parent() {
                        fs::create_dir_all(parent).await?;
                    }
                    fs::copy(&src_path, &dst_path).await?;
                } else if file_type.is_symlink() {
                    let link_target = fs::read_link(&src_path).await?;
                    if dst_path.exists() {
                        fs::remove_file(&dst_path).await?;
                    }
                    #[cfg(unix)]
                    tokio::fs::symlink(&link_target, &dst_path).await?;
                }
            }
        }

        Ok(())
    }

    async fn clear_directory(dir: &Path) -> Result<(), ShimError> {
        let mut entries = fs::read_dir(dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir() {
                fs::remove_dir_all(&path).await?;
            } else {
                fs::remove_file(&path).await?;
            }
        }
        Ok(())
    }
}

#[cfg(all(feature = "libkrun", target_os = "macos"))]
mod krun_impl {
    use super::*;
    use std::ffi::CString;
    use std::io::{BufRead, BufReader};
    use std::os::unix::io::{FromRawFd, RawFd};
    use std::process::{Command, Stdio};

    pub fn set_rlimits() {
        unsafe {
            let mut limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };

            if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) == 0 {
                limit.rlim_cur = limit.rlim_max;
                libc::setrlimit(libc::RLIMIT_NOFILE, &limit);
            }
        }
    }

    pub fn fix_root_mode(rootfs: &Path) {
        let _ = Command::new("xattr")
            .args(["-w", "user.containers.override_stat", "0:0:0755"])
            .arg(rootfs)
            .output();
    }

    /// Run a container in a forked child process.
    /// Returns (stdout_read_fd, child_pid) on success.
    pub fn fork_and_run_vm(
        rootfs_path: &Path,
        exec_path: &str,
        argv: &[String],
        env: &[String],
        workdir: Option<&str>,
    ) -> Result<(RawFd, libc::pid_t), ShimError> {
        // Create a pipe for stdout
        let mut stdout_pipe: [libc::c_int; 2] = [0, 0];
        if unsafe { libc::pipe(stdout_pipe.as_mut_ptr()) } != 0 {
            return Err(ShimError::RuntimeError("Failed to create pipe".to_string()));
        }

        let pid = unsafe { libc::fork() };
        
        if pid < 0 {
            return Err(ShimError::RuntimeError("Fork failed".to_string()));
        }

        if pid == 0 {
            // Child process
            unsafe {
                // Close read end of pipe
                libc::close(stdout_pipe[0]);
                
                // Redirect stdout/stderr to pipe write end
                libc::dup2(stdout_pipe[1], libc::STDOUT_FILENO);
                libc::dup2(stdout_pipe[1], libc::STDERR_FILENO);
                libc::close(stdout_pipe[1]);
            }

            // Run the VM in the child
            set_rlimits();
            
            let ctx_id = unsafe { krun_sys::krun_create_ctx() };
            if ctx_id < 0 {
                eprintln!("Failed to create context: {}", ctx_id);
                std::process::exit(1);
            }
            let ctx_id = ctx_id as u32;

            if unsafe { krun_sys::krun_set_vm_config(ctx_id, 2, 1100) } < 0 {
                eprintln!("Failed to set VM config");
                std::process::exit(1);
            }

            let root_cstr = CString::new(rootfs_path.to_string_lossy().as_bytes()).unwrap();
            if unsafe { krun_sys::krun_set_root(ctx_id, root_cstr.as_ptr()) } < 0 {
                eprintln!("Failed to set root");
                std::process::exit(1);
            }

            if let Some(wd) = workdir {
                let wd_cstr = CString::new(wd).unwrap();
                unsafe { krun_sys::krun_set_workdir(ctx_id, wd_cstr.as_ptr()) };
            }

            let exec_cstr = CString::new(exec_path).unwrap();
            let argv_cstrs: Vec<CString> = argv.iter()
                .map(|s| CString::new(s.as_bytes()).unwrap())
                .collect();
            let mut argv_ptrs: Vec<*const i8> = argv_cstrs.iter().map(|s| s.as_ptr()).collect();
            argv_ptrs.push(std::ptr::null());

            let env_cstrs: Vec<CString> = env.iter()
                .map(|s| CString::new(s.as_bytes()).unwrap())
                .collect();
            let mut env_ptrs: Vec<*const i8> = env_cstrs.iter().map(|s| s.as_ptr()).collect();
            env_ptrs.push(std::ptr::null());

            if unsafe { krun_sys::krun_set_exec(ctx_id, exec_cstr.as_ptr(), argv_ptrs.as_ptr(), env_ptrs.as_ptr()) } < 0 {
                eprintln!("Failed to set exec");
                std::process::exit(1);
            }

            let ret = unsafe { krun_sys::krun_start_enter(ctx_id) };
            std::process::exit(if ret == 0 { 0 } else { 1 });
        }

        // Parent process
        unsafe {
            // Close write end of pipe
            libc::close(stdout_pipe[1]);
        }

        Ok((stdout_pipe[0], pid))
    }

    /// Wait for child process and return exit code
    pub fn wait_for_child(pid: libc::pid_t) -> i32 {
        let mut status: libc::c_int = 0;
        unsafe {
            libc::waitpid(pid, &mut status, 0);
        }
        if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status)
        } else {
            1
        }
    }

    pub fn init_logging() {
        // Don't init logging in parent - child will handle it
    }
}

#[cfg(not(all(feature = "libkrun", target_os = "macos")))]
#[allow(dead_code)]
mod krun_impl {
    use super::*;

    pub struct KrunContext;

    impl KrunContext {
        pub fn new() -> Result<Self, ShimError> {
            Err(ShimError::NotSupported(
                "libkrun support not compiled in or not available on this platform".to_string(),
            ))
        }
    }

    pub fn init_logging() {}
}

#[async_trait]
impl Shim for KrunShim {
    async fn create(&self, opts: CreateContainerOpts) -> Result<String, ShimError> {
        let id = Uuid::new_v4().to_string();

        {
            let containers = self.containers.read().await;
            if containers.contains_key(&id) {
                return Err(ShimError::ContainerAlreadyExists(id));
            }
        }

        let bundle_path = self.data_dir.join("containers").join(&id).join("bundle");
        let rootfs_path = bundle_path.join("rootfs");
        fs::create_dir_all(&bundle_path).await?;

        // Prepare rootfs from snapshot mounts
        if !opts.mounts.is_empty() {
            tracing::info!(container_id = %id, "Preparing rootfs from {} mount(s)", opts.mounts.len());
            self.prepare_rootfs_from_mounts(&opts.mounts, &rootfs_path).await?;
        } else {
            // No mounts provided - create minimal rootfs structure
            tracing::info!(container_id = %id, "No mounts provided, creating minimal rootfs");
            rootfs::create_minimal_rootfs(&rootfs_path).await?;
        }

        let now = Self::current_timestamp();

        let info = ContainerInfo {
            id: id.clone(),
            name: opts.name.clone(),
            image: opts.config.image.clone(),
            state: ContainerState::Created,
            pid: None,
            exit_code: None,
            created_at: now,
            started_at: None,
            finished_at: None,
            bundle_path: bundle_path.to_string_lossy().to_string(),
            rootfs_path: rootfs_path.to_string_lossy().to_string(),
        };

        let metadata = ContainerMetadata {
            info,
            config: opts.config,
            host_config: opts.host_config,
        };

        self.save_container(&metadata).await?;

        {
            let mut containers = self.containers.write().await;
            containers.insert(id.clone(), metadata);
        }

        tracing::info!(container_id = %id, "Container created (libkrun)");
        Ok(id)
    }

    async fn start(&self, id: &str) -> Result<(), ShimError> {
        let mut containers = self.containers.write().await;
        let metadata = containers
            .get_mut(id)
            .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))?;

        if metadata.info.state != ContainerState::Created {
            return Err(ShimError::InvalidState {
                expected: "created".to_string(),
                actual: metadata.info.state.to_string(),
            });
        }

        metadata.info.state = ContainerState::Running;
        metadata.info.started_at = Some(Self::current_timestamp());
        self.save_container(metadata).await?;

        tracing::info!(container_id = %id, "Container started (libkrun)");
        Ok(())
    }

    async fn stop(&self, id: &str, _timeout: u32) -> Result<(), ShimError> {
        let mut containers = self.containers.write().await;
        let metadata = containers
            .get_mut(id)
            .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))?;

        if metadata.info.state != ContainerState::Running {
            return Err(ShimError::ContainerNotRunning(id.to_string()));
        }

        metadata.info.state = ContainerState::Stopped;
        metadata.info.finished_at = Some(Self::current_timestamp());
        metadata.info.pid = None;
        self.save_container(metadata).await?;

        tracing::info!(container_id = %id, "Container stopped (libkrun)");
        Ok(())
    }

    async fn kill(&self, id: &str, signal: u32) -> Result<(), ShimError> {
        let containers = self.containers.read().await;
        let metadata = containers
            .get(id)
            .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))?;

        if metadata.info.state != ContainerState::Running {
            return Err(ShimError::ContainerNotRunning(id.to_string()));
        }

        tracing::info!(container_id = %id, signal = signal, "Signal sent to container (libkrun)");
        Ok(())
    }

    async fn delete(&self, id: &str, force: bool) -> Result<(), ShimError> {
        {
            let containers = self.containers.read().await;
            let metadata = containers
                .get(id)
                .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))?;

            if metadata.info.state == ContainerState::Running && !force {
                return Err(ShimError::InvalidState {
                    expected: "stopped or created".to_string(),
                    actual: "running".to_string(),
                });
            }
        }

        let container_dir = self.data_dir.join("containers").join(id);
        if container_dir.exists() {
            fs::remove_dir_all(&container_dir).await?;
        }

        {
            let mut containers = self.containers.write().await;
            containers.remove(id);
        }

        tracing::info!(container_id = %id, "Container deleted (libkrun)");
        Ok(())
    }

    async fn pause(&self, id: &str) -> Result<(), ShimError> {
        let mut containers = self.containers.write().await;
        let metadata = containers
            .get_mut(id)
            .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))?;

        if metadata.info.state != ContainerState::Running {
            return Err(ShimError::ContainerNotRunning(id.to_string()));
        }

        metadata.info.state = ContainerState::Paused;
        self.save_container(metadata).await?;

        tracing::info!(container_id = %id, "Container paused (libkrun)");
        Ok(())
    }

    async fn resume(&self, id: &str) -> Result<(), ShimError> {
        let mut containers = self.containers.write().await;
        let metadata = containers
            .get_mut(id)
            .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))?;

        if metadata.info.state != ContainerState::Paused {
            return Err(ShimError::InvalidState {
                expected: "paused".to_string(),
                actual: metadata.info.state.to_string(),
            });
        }

        metadata.info.state = ContainerState::Running;
        self.save_container(metadata).await?;

        tracing::info!(container_id = %id, "Container resumed (libkrun)");
        Ok(())
    }

    async fn list(&self) -> Result<Vec<ContainerInfo>, ShimError> {
        let containers = self.containers.read().await;
        Ok(containers.values().map(|m| m.info.clone()).collect())
    }

    async fn get(&self, id: &str) -> Result<ContainerInfo, ShimError> {
        let containers = self.containers.read().await;
        containers
            .get(id)
            .map(|m| m.info.clone())
            .ok_or_else(|| ShimError::ContainerNotFound(id.to_string()))
    }

    async fn wait(&self, id: &str) -> Result<WaitResult, ShimError> {
        loop {
            {
                let containers = self.containers.read().await;
                if let Some(metadata) = containers.get(id) {
                    if metadata.info.state == ContainerState::Stopped {
                        return Ok(WaitResult {
                            exit_code: metadata.info.exit_code.unwrap_or(0),
                            error: None,
                        });
                    }
                } else {
                    return Err(ShimError::ContainerNotFound(id.to_string()));
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }

    #[allow(unused_variables, unused_assignments)]
    fn run_streaming(&self, id: String) -> OutputEventStream {
        let containers = self.containers.clone();
        let data_dir = self.data_dir.clone();

        Box::pin(async_stream::try_stream! {
            let (config, rootfs_path): (ContainerConfig, PathBuf);
            {
                let mut containers_guard = containers.write().await;
                let metadata = containers_guard
                    .get_mut(&id)
                    .ok_or_else(|| ShimError::ContainerNotFound(id.clone()))?;

                if metadata.info.state != ContainerState::Created {
                    Err(ShimError::InvalidState {
                        expected: "created".to_string(),
                        actual: metadata.info.state.to_string(),
                    })?;
                }

                config = metadata.config.clone();
                rootfs_path = PathBuf::from(&metadata.info.rootfs_path);

                metadata.info.state = ContainerState::Running;
                metadata.info.started_at = Some(KrunShim::current_timestamp());

                let container_dir = data_dir.join("containers").join(&metadata.info.id);
                fs::create_dir_all(&container_dir).await?;
                let metadata_path = container_dir.join("metadata.json");
                let content = serde_json::to_string_pretty(&metadata)?;
                fs::write(&metadata_path, content).await?;
            }

            tracing::info!(container_id = %id, rootfs = ?rootfs_path, "Starting container with libkrun (streaming)");

            #[cfg(all(feature = "libkrun", target_os = "macos"))]
            {
                krun_impl::fix_root_mode(&rootfs_path);

                let (exec_path, argv) = if !config.entrypoint.is_empty() {
                    let mut args = config.entrypoint.clone();
                    args.extend(config.cmd.clone());
                    (config.entrypoint[0].clone(), args)
                } else if !config.cmd.is_empty() {
                    (config.cmd[0].clone(), config.cmd.clone())
                } else {
                    ("/bin/sh".to_string(), vec!["/bin/sh".to_string()])
                };

                let workdir = config.working_dir.as_deref();

                // Fork and run VM in child process
                let (stdout_fd, child_pid) = krun_impl::fork_and_run_vm(
                    &rootfs_path,
                    &exec_path,
                    &argv,
                    &config.env,
                    workdir,
                )?;

                // Read output from child's stdout pipe
                let stdout_file = unsafe { std::fs::File::from_raw_fd(stdout_fd) };
                let mut reader = std::io::BufReader::new(stdout_file);
                
                // Spawn a task to read output
                let (output_tx, mut output_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
                let containers_for_reader = containers.clone();
                let id_for_reader = id.clone();
                
                std::thread::spawn(move || {
                    use std::io::Read;
                    let mut buf = [0u8; 4096];
                    loop {
                        match reader.get_mut().read(&mut buf) {
                            Ok(0) => break, // EOF
                            Ok(n) => {
                                if output_tx.blocking_send(buf[..n].to_vec()).is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                });

                // Wait for child in another thread and update state
                let containers_for_wait = containers.clone();
                let id_for_wait = id.clone();
                let data_dir_for_wait = data_dir.clone();
                
                tokio::task::spawn_blocking(move || {
                    let exit_code = krun_impl::wait_for_child(child_pid);
                    
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();

                    rt.block_on(async {
                        let mut containers_guard = containers_for_wait.write().await;
                        if let Some(metadata) = containers_guard.get_mut(&id_for_wait) {
                            metadata.info.state = ContainerState::Stopped;
                            metadata.info.exit_code = Some(exit_code);
                            metadata.info.finished_at = Some(KrunShim::current_timestamp());

                            let container_dir = data_dir_for_wait.join("containers").join(&id_for_wait);
                            let _ = fs::create_dir_all(&container_dir).await;
                            let metadata_path = container_dir.join("metadata.json");
                            if let Ok(content) = serde_json::to_string_pretty(&metadata) {
                                let _ = fs::write(&metadata_path, content).await;
                            }
                        }
                    });
                });

                // Stream output to client
                while let Some(data) = output_rx.recv().await {
                    yield OutputEvent::Stdout(data);
                }

                // Wait a bit for state to be updated
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                let exit_code = {
                    let containers_guard = containers.read().await;
                    containers_guard
                        .get(&id)
                        .and_then(|m| m.info.exit_code)
                        .unwrap_or(0)
                };

                yield OutputEvent::Exit(WaitResult {
                    exit_code,
                    error: None,
                });
            }

            #[cfg(not(all(feature = "libkrun", target_os = "macos")))]
            {
                yield OutputEvent::Exit(WaitResult {
                    exit_code: 1,
                    error: Some("libkrun support not available".to_string()),
                });
            }
        })
    }

    #[allow(unused_variables, unused_assignments, unused_mut)]
    async fn run_interactive(
        &self,
        id: String,
        mut input_rx: tokio::sync::mpsc::Receiver<InputEvent>,
        output_tx: tokio::sync::mpsc::Sender<OutputEvent>,
    ) -> Result<(), ShimError> {
        let (config, rootfs_path): (ContainerConfig, PathBuf);
        {
            let mut containers = self.containers.write().await;
            let metadata = containers
                .get_mut(&id)
                .ok_or_else(|| ShimError::ContainerNotFound(id.clone()))?;

            if metadata.info.state != ContainerState::Created {
                return Err(ShimError::InvalidState {
                    expected: "created".to_string(),
                    actual: metadata.info.state.to_string(),
                });
            }

            config = metadata.config.clone();
            rootfs_path = PathBuf::from(&metadata.info.rootfs_path);

            metadata.info.state = ContainerState::Running;
            metadata.info.started_at = Some(Self::current_timestamp());
            self.save_container(metadata).await?;
        }

        tracing::info!(container_id = %id, rootfs = ?rootfs_path, "Starting container with libkrun (interactive)");

        #[cfg(all(feature = "libkrun", target_os = "macos"))]
        {
            krun_impl::fix_root_mode(&rootfs_path);

            let (exec_path, argv) = if !config.entrypoint.is_empty() {
                let mut args = config.entrypoint.clone();
                args.extend(config.cmd.clone());
                (config.entrypoint[0].clone(), args)
            } else if !config.cmd.is_empty() {
                (config.cmd[0].clone(), config.cmd.clone())
            } else {
                ("/bin/sh".to_string(), vec!["/bin/sh".to_string()])
            };

            let workdir = config.working_dir.as_deref();

            // Fork and run VM in child process
            let (stdout_fd, child_pid) = krun_impl::fork_and_run_vm(
                &rootfs_path,
                &exec_path,
                &argv,
                &config.env,
                workdir,
            )?;

            // Consume input (not used currently - would need vsock for bidirectional)
            tokio::spawn(async move {
                while input_rx.recv().await.is_some() {}
            });

            // Read output from child's stdout pipe
            let stdout_file = unsafe { std::fs::File::from_raw_fd(stdout_fd) };
            let mut reader = std::io::BufReader::new(stdout_file);

            let containers_clone = self.containers.clone();
            let data_dir_clone = self.data_dir.clone();
            let id_clone = id.clone();

            // Read and forward output
            let output_task = tokio::task::spawn_blocking(move || {
                use std::io::Read;
                let mut output_data = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    match reader.get_mut().read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => output_data.extend_from_slice(&buf[..n]),
                        Err(_) => break,
                    }
                }
                output_data
            });

            // Wait for child
            let exit_code = tokio::task::spawn_blocking(move || {
                krun_impl::wait_for_child(child_pid)
            }).await.unwrap_or(1);

            // Get output
            if let Ok(output_data) = output_task.await {
                if !output_data.is_empty() {
                    let _ = output_tx.send(OutputEvent::Stdout(output_data)).await;
                }
            }

            // Update container state
            {
                let mut containers_guard = self.containers.write().await;
                if let Some(metadata) = containers_guard.get_mut(&id) {
                    metadata.info.state = ContainerState::Stopped;
                    metadata.info.exit_code = Some(exit_code);
                    metadata.info.finished_at = Some(Self::current_timestamp());
                    self.save_container(metadata).await?;
                }
            }

            let _ = output_tx
                .send(OutputEvent::Exit(WaitResult {
                    exit_code,
                    error: None,
                }))
                .await;
        }

        #[cfg(not(all(feature = "libkrun", target_os = "macos")))]
        {
            let _ = input_rx;
            let _ = output_tx
                .send(OutputEvent::Exit(WaitResult {
                    exit_code: 1,
                    error: Some("libkrun support not available".to_string()),
                }))
                .await;
        }

        Ok(())
    }
}
