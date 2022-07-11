use crate::core::{client::Session, receiver::Request, Command, StatusResponse};

impl Session {
    pub async fn handle_noop(&mut self, request: Request, is_check: bool) -> Result<(), ()> {
        self.write_bytes(
            StatusResponse::completed(
                if !is_check {
                    Command::Noop
                } else {
                    Command::Check
                },
                request.tag,
            )
            .into_bytes(),
        )
        .await
    }
}
