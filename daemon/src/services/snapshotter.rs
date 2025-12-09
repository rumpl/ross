use ross_core::snapshotter_service_server::SnapshotterService;
use ross_core::*;
use ross_snapshotter::OverlaySnapshotter;
use std::sync::Arc;
use tonic::{Request, Response, Status};

pub struct SnapshotterServiceGrpc {
    snapshotter: Arc<OverlaySnapshotter>,
}

impl SnapshotterServiceGrpc {
    pub fn new(snapshotter: Arc<OverlaySnapshotter>) -> Self {
        Self { snapshotter }
    }
}

fn kind_to_proto(kind: ross_snapshotter::SnapshotKind) -> i32 {
    match kind {
        ross_snapshotter::SnapshotKind::View => SnapshotKind::View as i32,
        ross_snapshotter::SnapshotKind::Active => SnapshotKind::Active as i32,
        ross_snapshotter::SnapshotKind::Committed => SnapshotKind::Committed as i32,
    }
}

fn info_to_grpc(info: &ross_snapshotter::SnapshotInfo) -> SnapshotInfo {
    SnapshotInfo {
        key: info.key.clone(),
        parent: info.parent.clone().unwrap_or_default(),
        kind: kind_to_proto(info.kind),
        created_at: Some(prost_types::Timestamp {
            seconds: info.created_at,
            nanos: 0,
        }),
        updated_at: Some(prost_types::Timestamp {
            seconds: info.updated_at,
            nanos: 0,
        }),
        labels: info.labels.clone(),
    }
}

fn mount_to_grpc(mount: &ross_snapshotter::Mount) -> SnapshotMount {
    SnapshotMount {
        r#type: mount.mount_type.clone(),
        source: mount.source.clone(),
        target: mount.target.clone(),
        options: mount.options.clone(),
    }
}

#[tonic::async_trait]
impl SnapshotterService for SnapshotterServiceGrpc {
    async fn prepare(
        &self,
        request: Request<PrepareSnapshotRequest>,
    ) -> Result<Response<PrepareSnapshotResponse>, Status> {
        let req = request.into_inner();

        let parent = if req.parent.is_empty() {
            None
        } else {
            Some(req.parent.as_str())
        };

        let mounts = self
            .snapshotter
            .prepare(&req.key, parent, req.labels)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(PrepareSnapshotResponse {
            mounts: mounts.iter().map(mount_to_grpc).collect(),
        }))
    }

    async fn view(
        &self,
        request: Request<ViewSnapshotRequest>,
    ) -> Result<Response<ViewSnapshotResponse>, Status> {
        let req = request.into_inner();

        let parent = if req.parent.is_empty() {
            None
        } else {
            Some(req.parent.as_str())
        };

        let mounts = self
            .snapshotter
            .view(&req.key, parent, req.labels)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(ViewSnapshotResponse {
            mounts: mounts.iter().map(mount_to_grpc).collect(),
        }))
    }

    async fn mounts(
        &self,
        request: Request<SnapshotMountsRequest>,
    ) -> Result<Response<SnapshotMountsResponse>, Status> {
        let req = request.into_inner();

        let mounts = self
            .snapshotter
            .mounts(&req.key)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(SnapshotMountsResponse {
            mounts: mounts.iter().map(mount_to_grpc).collect(),
        }))
    }

    async fn commit(
        &self,
        request: Request<CommitSnapshotRequest>,
    ) -> Result<Response<CommitSnapshotResponse>, Status> {
        let req = request.into_inner();

        self.snapshotter
            .commit(&req.key, &req.active_key, req.labels)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(CommitSnapshotResponse {}))
    }

    async fn remove(
        &self,
        request: Request<RemoveSnapshotRequest>,
    ) -> Result<Response<RemoveSnapshotResponse>, Status> {
        let req = request.into_inner();

        self.snapshotter
            .remove(&req.key)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(RemoveSnapshotResponse {}))
    }

    async fn stat(
        &self,
        request: Request<StatSnapshotRequest>,
    ) -> Result<Response<StatSnapshotResponse>, Status> {
        let req = request.into_inner();

        let info = self
            .snapshotter
            .stat(&req.key)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(StatSnapshotResponse {
            info: Some(info_to_grpc(&info)),
        }))
    }

    async fn list(
        &self,
        request: Request<ListSnapshotsRequest>,
    ) -> Result<Response<ListSnapshotsResponse>, Status> {
        let req = request.into_inner();

        let parent_filter = if req.parent_filter.is_empty() {
            None
        } else {
            Some(req.parent_filter.as_str())
        };

        let infos = self
            .snapshotter
            .list(parent_filter)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(ListSnapshotsResponse {
            infos: infos.iter().map(info_to_grpc).collect(),
        }))
    }

    async fn usage(
        &self,
        request: Request<SnapshotUsageRequest>,
    ) -> Result<Response<SnapshotUsageResponse>, Status> {
        let req = request.into_inner();

        let usage = self
            .snapshotter
            .usage(&req.key)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(SnapshotUsageResponse {
            size: usage.size,
            inodes: usage.inodes,
        }))
    }

    async fn cleanup(
        &self,
        _request: Request<CleanupSnapshotsRequest>,
    ) -> Result<Response<CleanupSnapshotsResponse>, Status> {
        let reclaimed = self
            .snapshotter
            .cleanup()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(CleanupSnapshotsResponse {
            reclaimed_bytes: reclaimed,
        }))
    }

    async fn extract_layer(
        &self,
        request: Request<ExtractLayerRequest>,
    ) -> Result<Response<ExtractLayerResponse>, Status> {
        let req = request.into_inner();

        let parent_key = if req.parent_key.is_empty() {
            None
        } else {
            Some(req.parent_key.as_str())
        };

        let (key, size) = self
            .snapshotter
            .extract_layer(&req.digest, parent_key, &req.key, req.labels)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(ExtractLayerResponse { key, size }))
    }
}
