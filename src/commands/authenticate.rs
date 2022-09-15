/*
 * Copyright (c) 2020-2022, Stalwart Labs Ltd.
 *
 * This file is part of the Stalwart IMAP Server.
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of
 * the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 * in the LICENSE file at the top-level directory of this distribution.
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/>.
 *
 * You can be released from the requirements of the AGPLv3 license by
 * purchasing a commercial license. Please contact licensing@stalw.art
 * for more details.
*/

use std::sync::Arc;

use jmap_client::client::{Client, Credentials};
use tracing::debug;

use crate::{
    core::{
        client::{Session, SessionData, State},
        receiver::{self, Request},
        Command, ResponseCode, StatusResponse,
    },
    protocol::{authenticate::Mechanism, capability::Capability},
};

impl Session {
    pub async fn handle_authenticate(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_authenticate() {
            Ok(mut args) => match args.mechanism {
                Mechanism::Plain | Mechanism::OAuthBearer => {
                    if !args.params.is_empty() {
                        match base64::decode(&args.params.pop().unwrap()) {
                            Ok(challenge) => {
                                let result = if args.mechanism == Mechanism::Plain {
                                    decode_challenge_plain(&challenge)
                                } else {
                                    decode_challenge_oauth(&challenge)
                                };

                                match result {
                                    Ok(credentials) => {
                                        self.authenticate(credentials, args.tag).await
                                    }
                                    Err(err) => {
                                        self.write_bytes(
                                            StatusResponse::no(err).with_tag(args.tag).into_bytes(),
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
                            tokens: vec![receiver::Token::Argument(args.mechanism.into_bytes())],
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
        match Client::new()
            .follow_redirects(&self.core.trusted_hosts)
            .forwarded_for(self.peer_addr.ip())
            .credentials(credentials)
            .connect(&self.core.jmap_url)
            .await
        {
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
                        client,
                        core: self.core.clone(),
                        writer: self.writer.clone(),
                    }),
                };
                self.write_bytes(
                    StatusResponse::ok("Authentication successful")
                        .with_code(ResponseCode::Capability {
                            capabilities: Capability::all_capabilities(true, self.is_tls),
                        })
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

    pub async fn handle_unauthenticate(&mut self, request: Request) -> Result<(), ()> {
        self.state = State::NotAuthenticated { auth_failures: 0 };

        self.write_bytes(
            StatusResponse::completed(Command::Unauthenticate)
                .with_tag(request.tag)
                .into_bytes(),
        )
        .await
    }
}

fn decode_challenge_plain(challenge: &[u8]) -> Result<Credentials, &'static str> {
    let mut username = Vec::new();
    let mut secret = Vec::new();
    let mut arg_num = 0;
    for &ch in challenge {
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
        (Ok(username), Ok(secret)) if !username.is_empty() && !secret.is_empty() => {
            Ok((username, secret).into())
        }
        _ => Err("Invalid AUTH=PLAIN challenge."),
    }
}

fn decode_challenge_oauth(challenge: &[u8]) -> Result<Credentials, &'static str> {
    let mut saw_marker = true;
    for (pos, &ch) in challenge.iter().enumerate() {
        if saw_marker {
            if challenge
                .get(pos..)
                .map_or(false, |b| b.starts_with(b"auth=Bearer "))
            {
                let pos = pos + 12;
                return Ok(Credentials::Bearer(
                    String::from_utf8(
                        challenge
                            .get(
                                pos..pos
                                    + challenge
                                        .get(pos..)
                                        .and_then(|c| c.iter().position(|&ch| ch == 0x01))
                                        .unwrap_or(challenge.len()),
                            )
                            .ok_or("Failed to find end of bearer token")?
                            .to_vec(),
                    )
                    .map_err(|_| "Bearer token is not a valid UTF-8 string.")?,
                ));
            } else {
                saw_marker = false;
            }
        } else if ch == 0x01 {
            saw_marker = true;
        }
    }

    Err("Failed to find 'auth=Bearer' in challenge.")
}

#[cfg(test)]
mod tests {
    use jmap_client::client::Credentials;

    #[test]
    fn decode_challenge_oauth() {
        assert_eq!(
            Credentials::Bearer("vF9dft4qmTc2Nvb3RlckBhbHRhdmlzdGEuY29tCg==".to_string()),
            super::decode_challenge_oauth(
                &base64::decode(
                    concat!(
                        "bixhPXVzZXJAZXhhbXBsZS5jb20sAWhv",
                        "c3Q9c2VydmVyLmV4YW1wbGUuY29tAXBvcnQ9MTQzAWF1dGg9QmVhcmVyI",
                        "HZGOWRmdDRxbVRjMk52YjNSbGNrQmhiSFJoZG1semRHRXVZMjl0Q2c9PQ",
                        "EB"
                    )
                    .as_bytes(),
                )
                .unwrap(),
            )
            .unwrap()
        );
    }
}
