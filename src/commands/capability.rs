use crate::{
    core::{client::Session, receiver::Request},
    protocol::{
        authenticate::Mechanism,
        capability::{Capability, Response},
        ImapResponse,
    },
};

impl Session {
    pub async fn handle_capability(&mut self, request: Request) -> Result<(), ()> {
        let mut capabilities = Vec::with_capacity(5);
        capabilities.push(Capability::IMAP4rev2);
        capabilities.push(Capability::IMAP4rev1);
        if self.is_tls {
            capabilities.push(Capability::Auth(Mechanism::Plain));
        } else {
            capabilities.push(Capability::StartTLS);
            capabilities.push(Capability::LoginDisabled);
        }
        self.write_bytes(Response { capabilities }.serialize(request.tag))
            .await
    }
}
