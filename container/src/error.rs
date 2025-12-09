use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContainerError {
    #[error("container not found: {0}")]
    NotFound(String),

    #[error("container already exists: {0}")]
    AlreadyExists(String),

    #[error("container not running: {0}")]
    NotRunning(String),

    #[error("container already running: {0}")]
    AlreadyRunning(String),

    #[error("exec not found: {0}")]
    ExecNotFound(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("image not found: {0}")]
    ImageNotFound(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("shim error: {0}")]
    Shim(#[from] ross_shim::ShimError),

    #[error("snapshotter error: {0}")]
    Snapshotter(#[from] ross_snapshotter::SnapshotterError),

    #[error("store error: {0}")]
    Store(#[from] ross_store::StoreError),
}
