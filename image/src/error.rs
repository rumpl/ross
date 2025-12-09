use thiserror::Error;

#[derive(Error, Debug)]
pub enum ImageError {
    #[error("image not found: {0}")]
    NotFound(String),

    #[error("invalid reference: {0}")]
    InvalidReference(String),

    #[error("pull failed: {0}")]
    PullFailed(String),

    #[error("push failed: {0}")]
    PushFailed(String),

    #[error("build failed: {0}")]
    BuildFailed(String),

    #[error("registry error: {0}")]
    Registry(#[from] ross_remote::RegistryError),

    #[error("store error: {0}")]
    Store(#[from] ross_store::StoreError),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
