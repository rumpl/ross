mod error;
mod overlay;

pub use error::MountError;
pub use overlay::{mount_overlay, unmount};

#[derive(Debug, Clone)]
pub struct MountSpec {
    pub mount_type: String,
    pub source: String,
    pub options: Vec<String>,
}

impl MountSpec {
    pub fn new(mount_type: &str, source: &str, options: Vec<String>) -> Self {
        Self {
            mount_type: mount_type.to_string(),
            source: source.to_string(),
            options,
        }
    }
}
