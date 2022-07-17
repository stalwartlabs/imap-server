use crate::{
    core::{client::Session, receiver::Request},
    protocol::{namespace::Response, ImapResponse},
};

impl Session {
    pub async fn handle_namespace(&mut self, request: Request) -> Result<(), ()> {
        self.write_bytes(
            Response {
                shared_prefix: if self.state.session_data().mailboxes.lock().len() > 1 {
                    self.core.folder_shared.clone().into()
                } else {
                    None
                },
            }
            .serialize(request.tag),
        )
        .await
    }
}
