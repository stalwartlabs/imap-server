use std::sync::Arc;

use jmap_client::email::Property;

use crate::{
    core::{
        client::{Session, SessionData},
        message::MailboxData,
        receiver::Request,
        Command, Flag, IntoStatusResponse, ResponseCode, StatusResponse,
    },
    protocol::{
        fetch::{DataItem, FetchItem},
        store::{self, Arguments, Operation},
        ImapResponse,
    },
};

impl Session {
    pub async fn handle_store(&mut self, request: Request, is_uid: bool) -> Result<(), ()> {
        match request.parse_store() {
            Ok(arguments) => {
                let (data, mailbox, _) = self.state.mailbox_data();
                let version = self.version;

                tokio::spawn(async move {
                    let bytes = match data.store(arguments, mailbox, is_uid).await {
                        Ok((response, tag)) => response.serialize(tag, version),
                        Err(response) => response.into_bytes(),
                    };
                    data.write_bytes(bytes).await;
                });
                Ok(())
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}

impl SessionData {
    pub async fn store(
        &self,
        arguments: Arguments,
        mailbox: Arc<MailboxData>,
        is_uid: bool,
    ) -> Result<(store::Response, String), StatusResponse> {
        let mut request = self.client.build();
        let set_request = request.set_email().account_id(&mailbox.account_id);
        let keywords = arguments
            .keywords
            .iter()
            .map(|k| k.to_jmap())
            .collect::<Vec<_>>();
        let jmap_ids = match self
            .imap_sequence_to_jmap(mailbox.clone(), arguments.sequence_set, is_uid)
            .await
        {
            Ok(jmap_ids) => {
                if jmap_ids.is_empty() {
                    return Err(StatusResponse::completed(
                        Command::Store(is_uid),
                        arguments.tag,
                    ));
                }
                jmap_ids
            }
            Err(response) => {
                return Err(response.with_tag(arguments.tag));
            }
        };
        for jmap_id in jmap_ids.iter() {
            let update_item = set_request.update(jmap_id);
            let is_set = match arguments.operation {
                Operation::Set => {
                    update_item.keywords(arguments.keywords.iter().map(|k| k.to_jmap()));
                    continue;
                }
                Operation::Add => true,
                Operation::Clear => false,
            };
            for keyword in &keywords {
                update_item.keyword(keyword, is_set);
            }
        }

        if !arguments.is_silent {
            request
                .get_email()
                .account_id(&mailbox.account_id)
                .ids(jmap_ids.iter())
                .properties([Property::Id, Property::Keywords]);
        }

        match request.send().await {
            Ok(response) => {
                let mut response = response.unwrap_method_responses();
                if (arguments.is_silent && response.len() != 1)
                    || (!arguments.is_silent && response.len() != 2)
                {
                    return Err(StatusResponse::no(
                        arguments.tag.into(),
                        ResponseCode::ContactAdmin.into(),
                        "Invalid response received from JMAP server.",
                    ));
                }

                let get_response = if !arguments.is_silent {
                    match response.pop().unwrap().unwrap_get_email() {
                        Ok(get_response) => get_response.into(),
                        Err(err) => {
                            return Err(err.into_status_response(arguments.tag.into()));
                        }
                    }
                } else {
                    None
                };

                if let Err(err) = response.pop().unwrap().unwrap_set_email() {
                    return Err(err.into_status_response(arguments.tag.into()));
                }

                if let Some(mut get_response) = get_response {
                    let mut flags = Vec::with_capacity(get_response.list().len());
                    let mut jmap_ids = Vec::with_capacity(get_response.list().len());
                    for email in get_response.unwrap_list() {
                        flags.push(DataItem::Flags {
                            flags: email
                                .keywords()
                                .iter()
                                .map(|k| Flag::parse_jmap(k.to_string()))
                                .collect(),
                        });
                        let jmap_id = email.unwrap_id();
                        if !jmap_id.is_empty() {
                            jmap_ids.push(jmap_id);
                        } else {
                            flags.pop();
                        }
                    }

                    match self
                        .core
                        .jmap_to_imap(mailbox, jmap_ids, true, is_uid)
                        .await
                    {
                        Ok((ids, _)) => Ok((
                            store::Response {
                                is_uid,
                                items: ids
                                    .into_iter()
                                    .zip(flags)
                                    .map(|(id, flags)| FetchItem {
                                        id,
                                        items: vec![flags],
                                    })
                                    .collect(),
                            },
                            arguments.tag,
                        )),
                        Err(_) => Err(StatusResponse::database_failure(arguments.tag.into())),
                    }
                } else {
                    Err(StatusResponse::completed(
                        Command::Store(is_uid),
                        arguments.tag,
                    ))
                }
            }
            Err(err) => Err(err.into_status_response(arguments.tag.into())),
        }
    }
}
