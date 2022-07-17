use crate::core::{client::Session, receiver::Request, Command, StatusResponse};

impl Session {
    pub async fn handle_logout(&mut self, request: Request) -> Result<(), ()> {
        let mut response = StatusResponse::bye(
            None,
            None,
            concat!(
                "Stalwart IMAP4rev2 v",
                env!("CARGO_PKG_VERSION"),
                " bids you farewell."
            )
            .to_string(),
        )
        .into_bytes();
        response.extend(StatusResponse::completed(Command::Logout, request.tag).into_bytes());
        self.write_bytes(response).await?;
        Err(())
    }
}