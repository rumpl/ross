use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotKind {
    View,
    Active,
    Committed,
}

impl std::fmt::Display for SnapshotKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotKind::View => write!(f, "view"),
            SnapshotKind::Active => write!(f, "active"),
            SnapshotKind::Committed => write!(f, "committed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotInfo {
    pub key: String,
    pub parent: Option<String>,
    pub kind: SnapshotKind,
    pub created_at: i64,
    pub updated_at: i64,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct Mount {
    pub mount_type: String,
    pub source: String,
    pub target: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Usage {
    pub size: i64,
    pub inodes: i64,
}
