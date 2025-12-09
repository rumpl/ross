use thiserror::Error;

#[derive(Error, Debug)]
pub enum RegistryError {
    #[error("invalid image reference: {0}")]
    InvalidReference(String),

    #[error("authentication required")]
    AuthRequired,

    #[error("authentication failed: {0}")]
    AuthFailed(String),

    #[error("manifest not found: {0}")]
    ManifestNotFound(String),

    #[error("blob not found: {0}")]
    BlobNotFound(String),

    #[error("unsupported media type: {0}")]
    UnsupportedMediaType(String),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("registry error: {0}")]
    Registry(String),
}
