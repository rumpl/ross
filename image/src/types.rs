use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct Image {
    pub id: String,
    pub repo_tags: Vec<String>,
    pub repo_digests: Vec<String>,
    pub parent: String,
    pub comment: String,
    pub container: String,
    pub docker_version: String,
    pub author: String,
    pub architecture: String,
    pub os: String,
    pub size: i64,
    pub virtual_size: i64,
    pub labels: HashMap<String, String>,
    pub root_fs: Option<RootFs>,
}

#[derive(Debug, Clone, Default)]
pub struct RootFs {
    pub fs_type: String,
    pub layers: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ImageHistory {
    pub id: String,
    pub created_by: String,
    pub tags: Vec<String>,
    pub size: i64,
    pub comment: String,
}

#[derive(Debug, Clone, Default)]
pub struct ListImagesParams {
    pub all: bool,
    pub filters: HashMap<String, String>,
    pub digests: bool,
}

#[derive(Debug, Clone)]
pub struct ImageInspection {
    pub image: Image,
    pub history: Vec<ImageHistory>,
}

#[derive(Debug, Clone)]
pub struct PullProgress {
    pub id: String,
    pub status: String,
    pub progress: String,
    pub current: Option<i64>,
    pub total: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PushProgress {
    pub id: String,
    pub status: String,
    pub progress: String,
    pub current: Option<i64>,
    pub total: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BuildProgress {
    pub stream: String,
    pub error: Option<String>,
    pub progress: String,
    pub aux_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct BuildParams {
    pub dockerfile: String,
    pub context_path: String,
    pub tags: Vec<String>,
    pub build_args: HashMap<String, String>,
    pub no_cache: bool,
    pub pull: bool,
    pub target: String,
    pub labels: HashMap<String, String>,
    pub platform: String,
}

#[derive(Debug, Clone)]
pub struct RemoveImageResult {
    pub deleted: Vec<String>,
    pub untagged: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub name: String,
    pub description: String,
    pub star_count: i32,
    pub is_official: bool,
    pub is_automated: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SearchParams {
    pub term: String,
    pub limit: i32,
    pub filters: HashMap<String, String>,
}

#[derive(Debug, Clone, Default)]
pub struct RegistryAuth {
    pub username: String,
    pub password: String,
    pub server_address: String,
    pub identity_token: String,
}
