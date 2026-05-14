use llamaforge_types::traits::{
    AdmissionError, IncomingRequest, QueueError, QueueSnapshot, QueueSubmitter, QueueTicket,
};

#[derive(Default)]
pub struct QueueSubmitterAdapter {}

impl QueueSubmitterAdapter {
    pub fn new() -> Self {
        Self {}
    }
}

impl QueueSubmitter for QueueSubmitterAdapter {
    fn submit(&self, _req: IncomingRequest) -> Result<QueueTicket, AdmissionError> {
        unimplemented!("Implemented in PR3")
    }

    fn cancel(&self, _id: &llamaforge_types::RequestId) -> Result<(), QueueError> {
        unimplemented!("Implemented in PR3")
    }

    fn status(&self) -> Result<QueueSnapshot, QueueError> {
        unimplemented!("Implemented in PR3")
    }
}
