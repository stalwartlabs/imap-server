use std::sync::Arc;

use jmap_client::client::{Client, Credentials};
use tracing::debug;

use crate::{
    core::{
        client::{Session, SessionData, State},
        receiver::{self, Request},
        Command, StatusResponse,
    },
    parser::authenticate::parse_authenticate,
    protocol::authenticate::Mechanism,
};

impl Session {
    pub async fn handle_authenticate(&mut self, request: Request) -> Result<(), ()> {
        match parse_authenticate(request) {
            Ok(mut args) => match args.mechanism {
                Mechanism::Plain => {
                    if !args.params.is_empty() {
                        match base64::decode(&args.params.pop().unwrap()) {
                            Ok(credentials) => {
                                let mut username = Vec::new();
                                let mut secret = Vec::new();
                                let mut arg_num = 0;
                                for ch in credentials {
                                    if ch != 0 {
                                        if arg_num == 1 {
                                            username.push(ch);
                                        } else if arg_num == 2 {
                                            secret.push(ch);
                                        }
                                    } else {
                                        arg_num += 1;
                                    }
                                }

                                match (String::from_utf8(username), String::from_utf8(secret)) {
                                    (Ok(username), Ok(secret))
                                        if !username.is_empty() && !secret.is_empty() =>
                                    {
                                        self.authenticate(
                                            Credentials::basic(&username, &secret),
                                            args.tag,
                                        )
                                        .await
                                    }
                                    _ => {
                                        self.write_bytes(
                                            StatusResponse::no(
                                                args.tag.into(),
                                                None,
                                                "Invalid AUTH=PLAIN challenge.",
                                            )
                                            .into_bytes(),
                                        )
                                        .await
                                    }
                                }
                            }
                            Err(_) => {
                                self.write_bytes(
                                    StatusResponse::no(
                                        args.tag.into(),
                                        None,
                                        "Failed to decode challenge.",
                                    )
                                    .into_bytes(),
                                )
                                .await
                            }
                        }
                    } else {
                        self.receiver.request = receiver::Request {
                            tag: args.tag,
                            command: Command::Authenticate,
                            tokens: vec![receiver::Token::Argument(b"PLAIN".to_vec())],
                        };
                        self.receiver.state = receiver::State::Argument { last_ch: b' ' };
                        self.write_bytes(b"+ \"\"\r\n".to_vec()).await
                    }
                }
                _ => {
                    self.write_bytes(
                        StatusResponse::no(
                            args.tag.into(),
                            None,
                            "Authentication mechanism not supported.",
                        )
                        .into_bytes(),
                    )
                    .await
                }
            },
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }

    pub async fn authenticate(&mut self, credentials: Credentials, tag: String) -> Result<(), ()> {
        match Client::connect(&self.config.jmap_url, credentials).await {
            Ok(client) => {
                self.state = State::Authenticated {
                    data: Arc::new(SessionData {
                        client,
                        config: self.config.clone(),
                        writer: self.writer.clone(),
                    }),
                };
                Ok(())
            }
            Err(err) => {
                debug!("Failed to connect to {}: {}", self.config.jmap_url, err,);
                self.write_bytes(
                    StatusResponse::no(tag.into(), None, "Authentication failed").into_bytes(),
                )
                .await?;

                let auth_failures = self.state.auth_failures();
                if auth_failures < 3 {
                    self.state = State::NotAuthenticated {
                        auth_failures: auth_failures + 1,
                    };
                    Ok(())
                } else {
                    self.write_bytes(
                        StatusResponse::bye(None, None, "Too many authentication failures")
                            .into_bytes(),
                    )
                    .await?;
                    debug!(
                        "Too many authentication failures, disconnecting {}",
                        self.peer_addr
                    );
                    Err(())
                }
            }
        }
    }
}
