use crate::core::{
    client::{Session, State},
    receiver::Request,
    Command, StatusResponse,
};

impl Session {
    pub async fn handle_noop(&mut self, request: Request, is_check: bool) -> Result<(), ()> {
        match &self.state {
            State::Authenticated { data } => {
                data.write_changes(None, true, false, self.version.is_rev2())
                    .await;
            }
            State::Selected { data, mailbox, .. } => {
                data.write_changes(mailbox.into(), true, true, self.version.is_rev2())
                    .await;
            }
            _ => (),
        }

        self.write_bytes(
            StatusResponse::completed(if !is_check {
                Command::Noop
            } else {
                Command::Check
            })
            .with_tag(request.tag)
            .into_bytes(),
        )
        .await
    }
}
