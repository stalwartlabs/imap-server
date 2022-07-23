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
        let mut capabilities = vec![
            Capability::IMAP4rev2,
            Capability::IMAP4rev1,
            Capability::Idle,
            Capability::Namespace,
            Capability::Id,
            Capability::Children,
            Capability::MultiAppend,
            Capability::Binary,
            Capability::Unselect,
            Capability::ACL,
            Capability::UIDPlus,
            Capability::ESearch,
            Capability::SASLIR,
            Capability::Within,
            Capability::Enable,
            Capability::SearchRes,
            Capability::Sort,
            Capability::Thread,
            Capability::ListExtended,
            Capability::ESort,
            Capability::SortDisplay,
            Capability::SpecialUse,
            Capability::CreateSpecialUse,
            Capability::Move,
            Capability::CondStore,
            Capability::QResync,
            Capability::UnAuthenticate,
            Capability::StatusSize,
            Capability::ObjectId,
            Capability::Preview,
            Capability::Auth(Mechanism::OAuthBearer),
            Capability::Auth(Mechanism::Plain),
        ];
        if !self.is_tls {
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
                        "\"support-url\" \"https://stalw.art/imap\")\r\n"
                    )
                    .as_bytes()
                    .to_vec(),
                ),
        )
        .await
    }
}
