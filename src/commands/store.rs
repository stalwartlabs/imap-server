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
        store::{self, Arguments, Operation, Response},
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
    ) -> Result<(store::Response<'_>, String), StatusResponse> {
        let max_objects_in_get = self
            .client
            .session()
            .core_capabilities()
            .map(|c| c.max_objects_in_get())
            .unwrap_or(500);
        let max_objects_in_set = self
            .client
            .session()
            .core_capabilities()
            .map(|c| c.max_objects_in_set())
            .unwrap_or(500);

        let keywords = arguments
            .keywords
            .iter()
            .map(|k| k.to_jmap())
            .collect::<Vec<_>>();

        // Convert IMAP ids to JMAP ids.
        let ids = match self
            .imap_sequence_to_jmap(mailbox.clone(), arguments.sequence_set, is_uid)
            .await
        {
            Ok(ids) => {
                if ids.uids.is_empty() {
                    return Err(StatusResponse::completed(
                        Command::Store(is_uid),
                        arguments.tag,
                    ));
                }
                ids
            }
            Err(response) => {
                return Err(response.with_tag(arguments.tag));
            }
        };

        // Update
        let mut request = self.client.build();
        let mut set_chunks = 0;
        let mut get_chunks = 0;
        for jmap_ids_chunk in ids.jmap_ids.chunks(max_objects_in_set) {
            let set_request = request.set_email().account_id(&mailbox.account_id);
            for jmap_id in jmap_ids_chunk {
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
                set_chunks += 1;
            }
        }

        if !arguments.is_silent {
            for jmap_ids_chunk in ids.jmap_ids.chunks(max_objects_in_get) {
                request
                    .get_email()
                    .account_id(&mailbox.account_id)
                    .ids(jmap_ids_chunk.iter())
                    .properties([Property::Id, Property::Keywords]);
                get_chunks += 1;
            }
        }

        match request.send().await {
            Ok(response) => {
                let mut response = response.unwrap_method_responses();
                if (arguments.is_silent && response.len() != set_chunks)
                    || (!arguments.is_silent && response.len() != (set_chunks + get_chunks))
                {
                    return Err(StatusResponse::no(
                        arguments.tag.into(),
                        ResponseCode::ContactAdmin.into(),
                        "Invalid response received from JMAP server.",
                    ));
                }

                let emails = if !arguments.is_silent && get_chunks > 0 {
                    let mut emails =
                        Vec::with_capacity(((get_chunks - 1) * max_objects_in_get) + 10);
                    for _ in 0..set_chunks {
                        match response.pop().unwrap().unwrap_get_email() {
                            Ok(mut get_response) => {
                                emails.extend(get_response.take_list());
                            }
                            Err(err) => {
                                return Err(err.into_status_response(arguments.tag.into()));
                            }
                        }
                    }
                    emails
                } else {
                    Vec::new()
                };

                for _ in 0..set_chunks {
                    if let Err(err) = response.pop().unwrap().unwrap_set_email() {
                        return Err(err.into_status_response(arguments.tag.into()));
                    }
                }

                if emails.is_empty() {
                    Ok((
                        Response {
                            is_uid,
                            items: emails
                                .into_iter()
                                .filter_map(|email| {
                                    FetchItem {
                                        id: *ids
                                            .jmap_ids
                                            .iter()
                                            .position(|id| id == email.id().unwrap_or(""))
                                            .and_then(|pos| {
                                                if is_uid {
                                                    ids.uids.get(pos)
                                                } else {
                                                    ids.seqnums
                                                        .as_ref()
                                                        .and_then(|ids| ids.get(pos))
                                                }
                                            })?,
                                        items: vec![DataItem::Flags {
                                            flags: email
                                                .keywords()
                                                .iter()
                                                .map(|k| Flag::parse_jmap(k.to_string()))
                                                .collect(),
                                        }],
                                    }
                                    .into()
                                })
                                .collect(),
                        },
                        arguments.tag,
                    ))
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
