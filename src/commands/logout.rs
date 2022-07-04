use crate::core::{client::Session, receiver::Request, StatusResponse};

impl Session {
    pub async fn handle_logout(&mut self, request: Request) -> Result<(), ()> {
        self.write_bytes(
            StatusResponse::ok(request.tag.into(), None, "Romanes eunt domus.").into_bytes(),
        )
        .await?;
        Err(())
    }
}
