use thiserror::Error;

#[derive(Error, Debug)]
pub enum SnapshotterError {
    #[error("snapshot not found: {0}")]
    NotFound(String),

    #[error("snapshot already exists: {0}")]
    AlreadyExists(String),

    #[error("invalid snapshot state: expected {expected}, got {actual}")]
    InvalidState { expected: String, actual: String },

    #[error("parent snapshot not found: {0}")]
    ParentNotFound(String),

    #[error("snapshot has dependents and cannot be removed: {0}")]
    HasDependents(String),

    #[error("layer extraction failed: {0}")]
    ExtractionFailed(String),

    #[error("mount failed: {0}")]
    MountFailed(String),

    #[error("unmount failed: {0}")]
    UnmountFailed(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("store error: {0}")]
    Store(#[from] ross_store::StoreError),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
