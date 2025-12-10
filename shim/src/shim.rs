use crate::error::ShimError;
use crate::types::*;
use async_trait::async_trait;
use std::pin::Pin;

pub type OutputEventStream =
    Pin<Box<dyn futures::Stream<Item = Result<OutputEvent, ShimError>> + Send>>;

#[async_trait]
pub trait Shim: Send + Sync {
    async fn create(&self, opts: CreateContainerOpts) -> Result<String, ShimError>;

    async fn start(&self, id: &str) -> Result<(), ShimError>;

    async fn stop(&self, id: &str, timeout: u32) -> Result<(), ShimError>;

    async fn kill(&self, id: &str, signal: u32) -> Result<(), ShimError>;

    async fn delete(&self, id: &str, force: bool) -> Result<(), ShimError>;

    async fn pause(&self, id: &str) -> Result<(), ShimError>;

    async fn resume(&self, id: &str) -> Result<(), ShimError>;

    async fn list(&self) -> Result<Vec<ContainerInfo>, ShimError>;

    async fn get(&self, id: &str) -> Result<ContainerInfo, ShimError>;

    async fn wait(&self, id: &str) -> Result<WaitResult, ShimError>;

    fn run_streaming(&self, id: String) -> OutputEventStream;

    async fn run_interactive(
        &self,
        id: String,
        input_rx: tokio::sync::mpsc::Receiver<InputEvent>,
        output_tx: tokio::sync::mpsc::Sender<OutputEvent>,
    ) -> Result<(), ShimError>;
}
