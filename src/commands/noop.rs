use crate::core::{client::Session, receiver::Request, StatusResponse};

impl Session {
    pub async fn handle_noop(&mut self, request: Request) -> Result<(), ()> {
        self.write_bytes(
            StatusResponse::ok(request.tag.into(), None, "No soup for you... next!").into_bytes(),
        )
        .await
    }
}
