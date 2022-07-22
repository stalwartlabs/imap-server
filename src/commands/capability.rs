use crate::{
    core::{client::Session, receiver::Request, Command, StatusResponse},
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
        self.write_bytes(
            StatusResponse::completed(Command::Capability)
                .with_tag(request.tag)
                .serialize(Response { capabilities }.serialize()),
        )
        .await
    }

    pub async fn handle_id(&mut self, request: Request) -> Result<(), ()> {
        self.write_bytes(
            StatusResponse::completed(Command::Id)
                .with_tag(request.tag)
                .serialize(
                    concat!(
                        "* ID (\"name\" \"Stalwart IMAP\" \"version\" \"",
                        env!("CARGO_PKG_VERSION"),
                        "\" \"vendor\" \"Stalwart Labs Ltd.\" ",
                        "\"support-url\" \"https://stalw.art/imap\")"
                    )
                    .as_bytes()
                    .to_vec(),
                ),
        )
        .await
    }
}
