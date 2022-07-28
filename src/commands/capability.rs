use crate::{
    core::{client::Session, receiver::Request, Command, StatusResponse},
    protocol::{
        capability::{Capability, Response},
        ImapResponse,
    },
};

impl Session {
    pub async fn handle_capability(&mut self, request: Request) -> Result<(), ()> {
        self.write_bytes(
            StatusResponse::completed(Command::Capability)
                .with_tag(request.tag)
                .serialize(
                    Response {
                        capabilities: Capability::all_capabilities(
                            self.state.is_authenticated(),
                            self.is_tls,
                        ),
                    }
                    .serialize(),
                ),
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
                        "\"support-url\" \"https://stalw.art/imap\")\r\n"
                    )
                    .as_bytes()
                    .to_vec(),
                ),
        )
        .await
    }
}
