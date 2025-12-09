use serde::{Deserialize, Serialize};

pub const MEDIA_TYPE_MANIFEST_V2: &str = "application/vnd.docker.distribution.manifest.v2+json";
pub const MEDIA_TYPE_MANIFEST_LIST: &str =
    "application/vnd.docker.distribution.manifest.list.v2+json";
pub const MEDIA_TYPE_OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
pub const MEDIA_TYPE_OCI_INDEX: &str = "application/vnd.oci.image.index.v1+json";
pub const MEDIA_TYPE_LAYER_GZIP: &str = "application/vnd.docker.image.rootfs.diff.tar.gzip";
pub const MEDIA_TYPE_OCI_LAYER_GZIP: &str = "application/vnd.oci.image.layer.v1.tar+gzip";
pub const MEDIA_TYPE_CONFIG: &str = "application/vnd.docker.container.image.v1+json";
pub const MEDIA_TYPE_OCI_CONFIG: &str = "application/vnd.oci.image.config.v1+json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestV2 {
    pub schema_version: i32,
    pub media_type: Option<String>,
    pub config: Descriptor,
    pub layers: Vec<Descriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestList {
    pub schema_version: i32,
    pub media_type: Option<String>,
    pub manifests: Vec<ManifestDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestDescriptor {
    pub media_type: String,
    pub digest: String,
    pub size: i64,
    pub platform: Option<Platform>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Platform {
    pub architecture: String,
    pub os: String,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(rename = "os.version", default)]
    pub os_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub media_type: String,
    pub digest: String,
    pub size: i64,
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub annotations: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageConfig {
    pub architecture: String,
    pub os: String,
    #[serde(default)]
    pub config: Option<ContainerConfig>,
    #[serde(default)]
    pub rootfs: Option<RootFs>,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerConfig {
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub domainname: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub cmd: Vec<String>,
    #[serde(default)]
    pub entrypoint: Vec<String>,
    #[serde(default)]
    pub working_dir: String,
    #[serde(default)]
    pub labels: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub exposed_ports: std::collections::HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub volumes: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootFs {
    #[serde(rename = "type")]
    pub fs_type: String,
    pub diff_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub empty_layer: Option<bool>,
    #[serde(default)]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub token: Option<String>,
    pub access_token: Option<String>,
    pub expires_in: Option<i64>,
}

impl TokenResponse {
    pub fn get_token(&self) -> Option<&str> {
        self.token.as_deref().or(self.access_token.as_deref())
    }
}

#[derive(Debug, Clone)]
pub enum Manifest {
    V2(ManifestV2),
    List(ManifestList),
}

impl Manifest {
    pub fn layers(&self) -> Option<&[Descriptor]> {
        match self {
            Manifest::V2(m) => Some(&m.layers),
            Manifest::List(_) => None,
        }
    }

    pub fn config(&self) -> Option<&Descriptor> {
        match self {
            Manifest::V2(m) => Some(&m.config),
            Manifest::List(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PullProgress {
    pub id: String,
    pub status: PullStatus,
    pub current: u64,
    pub total: u64,
}

#[derive(Debug, Clone)]
pub enum PullStatus {
    Resolving,
    Resolved { digest: String },
    Downloading,
    Downloaded,
    Extracting,
    Extracted,
    Exists,
    Error(String),
    Complete,
}

impl std::fmt::Display for PullStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PullStatus::Resolving => write!(f, "Resolving"),
            PullStatus::Resolved { digest } => write!(f, "Resolved: {}", digest),
            PullStatus::Downloading => write!(f, "Downloading"),
            PullStatus::Downloaded => write!(f, "Download complete"),
            PullStatus::Extracting => write!(f, "Extracting"),
            PullStatus::Extracted => write!(f, "Pull complete"),
            PullStatus::Exists => write!(f, "Already exists"),
            PullStatus::Error(e) => write!(f, "Error: {}", e),
            PullStatus::Complete => write!(f, "Complete"),
        }
    }
}
