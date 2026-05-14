use llamaforge_types::traits::{ServerControl, ServerError, ServerSummary};

#[derive(Default)]
pub struct ServerControlAdapter {}

impl ServerControlAdapter {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait::async_trait]
impl ServerControl for ServerControlAdapter {
    async fn list_servers(&self) -> Result<Vec<ServerSummary>, ServerError> {
        unimplemented!("Implemented in PR4")
    }

    async fn start_server(&self, _id: &llamaforge_types::ServerId) -> Result<(), ServerError> {
        unimplemented!("Implemented in PR4")
    }

    async fn stop_server(&self, _id: &llamaforge_types::ServerId) -> Result<(), ServerError> {
        unimplemented!("Implemented in PR4")
    }
}
