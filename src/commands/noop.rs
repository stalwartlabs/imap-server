use crate::core::{client::Session, receiver::Request, StatusResponse};

impl Session {
    pub async fn handle_noop(&mut self, request: Request, is_check: bool) -> Result<(), ()> {
        self.write_bytes(
            StatusResponse::ok(
                request.tag.into(),
                None,
                if !is_check {
                    "NOOP completed"
                } else {
                    "CHECK completed"
                },
            )
            .into_bytes(),
        )
        .await
    }
}
