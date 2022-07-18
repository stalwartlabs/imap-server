use crate::{
    core::{client::Session, receiver::Request, Command, StatusResponse},
    protocol::{namespace::Response, ImapResponse},
};

impl Session {
    pub async fn handle_namespace(&mut self, request: Request) -> Result<(), ()> {
        self.write_bytes(
            StatusResponse::completed(Command::Namespace)
                .with_tag(request.tag)
                .serialize(
                    Response {
                        shared_prefix: if self.state.session_data().mailboxes.lock().len() > 1 {
                            self.core.folder_shared.clone().into()
                        } else {
                            None
                        },
                    }
                    .serialize(),
                ),
        )
        .await
    }
}
