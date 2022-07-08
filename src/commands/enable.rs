use crate::{
    core::{client::Session, receiver::Request, StatusResponse},
    protocol::{capability::Capability, ProtocolVersion},
};

impl Session {
    pub async fn handle_enable(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_enable() {
            Ok(arguments) => {
                for capability in arguments.capabilities {
                    match capability {
                        Capability::IMAP4rev2 => {
                            self.version = ProtocolVersion::Rev2;
                        }
                        Capability::IMAP4rev1 => {
                            self.version = ProtocolVersion::Rev1;
                        }
                        _ => {
                            let mut buf = Vec::with_capacity(10);
                            capability.serialize(&mut buf);
                            self.write_bytes(
                                StatusResponse::ok(
                                    arguments.tag.into(),
                                    None,
                                    format!(
                                        "{} cannot be enabled.",
                                        String::from_utf8(buf).unwrap()
                                    ),
                                )
                                .into_bytes(),
                            )
                            .await?;
                            return Ok(());
                        }
                    }
                }

                self.write_bytes(
                    StatusResponse::ok(arguments.tag.into(), None, "ENABLE successful.")
                        .into_bytes(),
                )
                .await
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}
