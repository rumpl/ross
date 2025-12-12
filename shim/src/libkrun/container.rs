//! Container metadata and state management.

use crate::types::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerMetadata {
    pub info: ContainerInfo,
    pub config: ContainerConfig,
    pub host_config: HostConfig,
}

impl ContainerMetadata {
    pub async fn load(path: &Path) -> Result<Self, crate::ShimError> {
        let content = fs::read_to_string(path).await?;
        let metadata = serde_json::from_str(&content)?;
        Ok(metadata)
    }

    pub async fn save(&self, dir: &Path) -> Result<(), crate::ShimError> {
        fs::create_dir_all(dir).await?;
        let path = dir.join("metadata.json");
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&path, content).await?;
        Ok(())
    }
}
