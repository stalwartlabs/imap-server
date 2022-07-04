use jmap_client::client::{Client, Credentials};
use tracing::debug;

use crate::{
    core::{
        client::{Session, State},
        receiver::Request,
        StatusResponse,
    },
    parser::login::parse_login,
};

impl Session {
    pub async fn handle_login(&mut self, request: Request) -> Result<(), ()> {
        match parse_login(request.tokens) {
            Ok(args) => {
                match Client::connect(
                    &self.config.jmap_url,
                    Credentials::basic(&args.username, &args.password),
                )
                .await
                {
                    Ok(client) => {
                        //let ws = client.connect_ws().await?;
                        self.state = State::Authenticated { client };
                        Ok(())
                    }
                    Err(err) => {
                        debug!(
                            "Failed to connect to JMAP: {} (account {})",
                            err, args.username
                        );
                        self.write_bytes(
                            StatusResponse::no(request.tag.into(), None, "Authentication failed")
                                .into_bytes(),
                        )
                        .await
                    }
                }
            }
            Err(message) => {
                self.write_bytes(
                    StatusResponse::parse_error(request.tag.into(), message).into_bytes(),
                )
                .await
            }
        }
    }
}
