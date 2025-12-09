use thiserror::Error;

#[derive(Error, Debug)]
pub enum ShimError {
    #[error("container not found: {0}")]
    ContainerNotFound(String),

    #[error("container already exists: {0}")]
    ContainerAlreadyExists(String),

    #[error("container not running: {0}")]
    ContainerNotRunning(String),

    #[error("invalid container state: expected {expected}, got {actual}")]
    InvalidState { expected: String, actual: String },

    #[error("bundle preparation failed: {0}")]
    BundlePreparationFailed(String),

    #[error("runc error: {0}")]
    Runc(String),

    #[error("oci spec error: {0}")]
    OciSpec(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<runc::error::Error> for ShimError {
    fn from(e: runc::error::Error) -> Self {
        ShimError::Runc(e.to_string())
    }
}

impl From<oci_spec::OciSpecError> for ShimError {
    fn from(e: oci_spec::OciSpecError) -> Self {
        ShimError::OciSpec(e.to_string())
    }
}
