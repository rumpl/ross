use thiserror::Error;

#[derive(Error, Debug)]
pub enum MountError {
    #[error("mount failed: {0}")]
    MountFailed(String),

    #[error("unmount failed: {0}")]
    UnmountFailed(String),

    #[error("invalid mount specification: {0}")]
    InvalidSpec(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("system error: {0}")]
    System(#[from] nix::errno::Errno),
}
