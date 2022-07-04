use crate::{
    core::{client::Session, receiver::Request},
    protocol::{
        capability::{Capability, Response},
        ImapResponse, ProtocolVersion,
    },
};

impl Session {
    pub async fn handle_capability(&mut self, request: Request) -> Result<(), ()> {
        let mut capabilities = Vec::with_capacity(5);
        if !self.is_tls {
            capabilities.push(Capability::StartTLS);
            capabilities.push(Capability::LoginDisabled);
        }
        if self.version == ProtocolVersion::Rev1 {
            capabilities.push(Capability::IMAP4rev2);
        }
        self.write_bytes(Response { capabilities }.serialize(request.tag, self.version))
            .await
    }
}
