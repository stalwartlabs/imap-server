use std::sync::Arc;

use jmap_client::client::{Client, Credentials};
use tracing::debug;

use crate::{
    core::{
        client::{Session, SessionData, State},
        receiver::{self, Request},
        Command, ResponseCode, StatusResponse,
    },
    protocol::authenticate::Mechanism,
};

use super::search::SavedSearch;

impl Session {
    pub async fn handle_authenticate(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_authenticate() {
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
                                            StatusResponse::no("Invalid AUTH=PLAIN challenge.")
                                                .with_tag(args.tag)
                                                .into_bytes(),
                                        )
                                        .await
                                    }
                                }
                            }
                            Err(_) => {
                                self.write_bytes(
                                    StatusResponse::no("Failed to decode challenge.")
                                        .with_tag(args.tag)
                                        .with_code(ResponseCode::Parse)
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
                        StatusResponse::no("Authentication mechanism not supported.")
                            .with_tag(args.tag)
                            .with_code(ResponseCode::Cannot)
                            .into_bytes(),
                    )
                    .await
                }
            },
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }

    pub async fn authenticate(&mut self, credentials: Credentials, tag: String) -> Result<(), ()> {
        match Client::connect(&self.core.jmap_url, credentials).await {
            Ok(client) => {
                // Fetch mailboxes
                let mailboxes = self
                    .core
                    .fetch_mailboxes(&client, &self.core.folder_shared)
                    .await
                    .ok_or(())?;

                // Delete from cache mailboxes that no longer exist on the main account
                if self
                    .core
                    .purge_deleted_mailboxes(mailboxes.first().unwrap())
                    .await
                    .is_err()
                {
                    self.write_bytes(
                        StatusResponse::database_failure()
                            .with_tag(tag)
                            .into_bytes(),
                    )
                    .await?;
                    return Err(());
                }

                // Create session
                self.state = State::Authenticated {
                    data: Arc::new(SessionData {
                        mailboxes: parking_lot::Mutex::new(mailboxes),
                        saved_search: parking_lot::Mutex::new(SavedSearch::None),
                        client,
                        core: self.core.clone(),
                        writer: self.writer.clone(),
                    }),
                };
                self.write_bytes(
                    StatusResponse::ok("Authentication successful")
                        .with_tag(tag)
                        .into_bytes(),
                )
                .await?;
                Ok(())
            }
            Err(err) => {
                debug!("Failed to connect to {}: {}", self.core.jmap_url, err,);
                self.write_bytes(
                    StatusResponse::no("Authentication failed")
                        .with_tag(tag)
                        .with_code(ResponseCode::AuthenticationFailed)
                        .into_bytes(),
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
                        StatusResponse::bye("Too many authentication failures").into_bytes(),
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
