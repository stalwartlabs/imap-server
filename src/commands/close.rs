use crate::core::{
    client::{Session, State},
    receiver::Request,
    Command, StatusResponse,
};

impl Session {
    pub async fn handle_close(&mut self, request: Request) -> Result<(), ()> {
        let (data, mailbox, is_rw, _) = self.state.select_data();
        if is_rw {
            data.expunge(mailbox).await.ok();
        }

        self.state = State::Authenticated { data };
        self.write_bytes(
            StatusResponse::completed(Command::Close)
                .with_tag(request.tag)
                .into_bytes(),
        )
        .await
    }
}
