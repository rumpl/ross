use thiserror::Error;

#[derive(Error, Debug)]
pub enum StoreError {
    #[error("blob not found: {0}")]
    BlobNotFound(String),

    #[error("manifest not found: {0}")]
    ManifestNotFound(String),

    #[error("tag not found: {0}/{1}")]
    TagNotFound(String, String),

    #[error("digest mismatch: expected {expected}, got {actual}")]
    DigestMismatch { expected: String, actual: String },

    #[error("invalid digest format: {0}")]
    InvalidDigest(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl From<StoreError> for tonic::Status {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::BlobNotFound(_) | StoreError::ManifestNotFound(_) | StoreError::TagNotFound(_, _) => {
                tonic::Status::not_found(err.to_string())
            }
            StoreError::DigestMismatch { .. } | StoreError::InvalidDigest(_) => {
                tonic::Status::invalid_argument(err.to_string())
            }
            StoreError::Io(_) | StoreError::Serialization(_) => tonic::Status::internal(err.to_string()),
        }
    }
}
