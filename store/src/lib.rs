mod proto {
    tonic::include_proto!("ross.store");
}

mod error;
mod service;
mod storage;

pub use error::StoreError;
pub use proto::store_service_client::StoreServiceClient;
pub use proto::store_service_server::{StoreService, StoreServiceServer};
pub use proto::*;
pub use service::StoreServiceImpl;
pub use storage::FileSystemStore;
