use std::sync::Arc;

use jmap_client::{core::response::MethodResponse, email::Property};
use tracing::debug;

use crate::{
    core::{
        client::{Session, SessionData},
        message::{IdMappings, MailboxData},
        receiver::Request,
        Command, Flag, IntoStatusResponse, ResponseCode, ResponseType, StatusResponse,
    },
    protocol::{
        fetch::{DataItem, FetchItem},
        store::{Arguments, Operation, Response},
        ImapResponse,
    },
};

impl Session {
    pub async fn handle_store(&mut self, request: Request, is_uid: bool) -> Result<(), ()> {
        match request.parse_store() {
            Ok(arguments) => {
                let (data, mailbox, _, condstore) = self.state.select_data();
                let is_condstore = self.is_condstore || condstore;

                tokio::spawn(async move {
                    let bytes = match data.store(arguments, mailbox, is_uid, is_condstore).await {
                        Ok(response) => response,
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
        is_condstore: bool,
    ) -> Result<Vec<u8>, StatusResponse> {
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

        let sequence_set = if is_uid && arguments.unchanged_since.is_some() {
            arguments.sequence_set.try_expand()
        } else {
            None
        };

        // Convert IMAP ids to JMAP ids.
        let mut ids = match self
            .imap_sequence_to_jmap(mailbox.clone(), arguments.sequence_set, is_uid)
            .await
        {
            Ok(ids) => {
                if ids.uids.is_empty() {
                    return Err(
                        StatusResponse::completed(Command::Store(is_uid)).with_tag(arguments.tag)
                    );
                }
                ids
            }
            Err(response) => {
                return Err(response.with_tag(arguments.tag));
            }
        };

        // Filter out unchanged since ids
        let mut response_code = None;
        let mut unchanged_failed = false;
        if let Some(unchanged_since) = arguments.unchanged_since {
            // Convert MODSEQ to JMAP State
            let state = match self
                .core
                .modseq_to_state(&mailbox.account_id, unchanged_since as u32)
                .await
            {
                Ok(Some(state)) => state,
                Ok(None) => {
                    return Err(StatusResponse::bad(format!(
                        "MODSEQ '{}' does not exist.",
                        unchanged_since
                    ))
                    .with_tag(arguments.tag));
                }
                Err(_) => return Err(StatusResponse::database_failure().with_tag(arguments.tag)),
            };

            // Obtain changes since the modseq.
            let mut request = self.client.build();
            request.changes_email(state).account_id(&mailbox.account_id);
            match request.send_changes_email().await {
                Ok(changes) => {
                    let mut modified = Vec::new();
                    let mut jmap_ids = Vec::with_capacity(ids.jmap_ids.len());
                    let mut uids = Vec::with_capacity(ids.uids.len());
                    let mut seqnums = if !is_uid {
                        Vec::with_capacity(ids.uids.len()).into()
                    } else {
                        None
                    };

                    // Add all IDs that changed in this mailbox
                    for (pos, jmap_id) in ids.jmap_ids.iter().enumerate() {
                        let was_destroyed = if changes.destroyed().contains(jmap_id) {
                            unchanged_failed = true;
                            true
                        } else {
                            false
                        };
                        if changes.updated().contains(jmap_id)
                            || changes.created().contains(jmap_id)
                            || was_destroyed
                        {
                            modified.push(if is_uid {
                                ids.uids[pos]
                            } else {
                                ids.seqnums.as_ref().unwrap()[pos]
                            });
                        } else {
                            jmap_ids.push(jmap_id.clone());
                            uids.push(ids.uids[pos]);
                            if let (Some(seqnums), Some(changed_seqnums)) =
                                (&ids.seqnums, &mut seqnums)
                            {
                                changed_seqnums.push(seqnums[pos]);
                            }
                        }
                    }

                    // Add ids that were removed
                    if let Some(sequence_set) = sequence_set {
                        for uid in sequence_set {
                            if !uids.contains(&uid) && !ids.uids.contains(&uid) {
                                modified.push(uid);
                                unchanged_failed = true;
                            }
                        }
                    }

                    ids = Arc::new(IdMappings {
                        jmap_ids,
                        uids,
                        seqnums,
                    });
                    if !modified.is_empty() {
                        modified.sort_unstable();
                        response_code = ResponseCode::Modified { ids: modified }.into();
                    }
                }
                Err(err) => {
                    return Err(err.into_status_response().with_tag(arguments.tag));
                }
            }
        }

        // Build response
        let mut response = if !unchanged_failed {
            StatusResponse::completed(Command::Store(is_uid))
        } else {
            StatusResponse::no("Some of the messages no longer exist.")
        }
        .with_tag(arguments.tag);
        if let Some(response_code) = response_code {
            response = response.with_code(response_code)
        }
        if ids.jmap_ids.is_empty() {
            return Err(response);
        }

        // Update
        let mut request = self.client.build();
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
            }
        }

        if !arguments.is_silent {
            for jmap_ids_chunk in ids.jmap_ids.chunks(max_objects_in_get) {
                request
                    .get_email()
                    .account_id(&mailbox.account_id)
                    .ids(jmap_ids_chunk.iter())
                    .properties([Property::Id, Property::Keywords]);
            }
        }

        match request.send().await {
            Ok(set_response) => {
                let mut emails = Vec::new();
                let mut new_state = None;
                let mut updated_ids = Vec::new();
                for set_response in set_response.unwrap_method_responses() {
                    match set_response.unwrap_method_response() {
                        MethodResponse::GetEmail(mut set_response) => {
                            emails.extend(set_response.take_list());
                        }
                        MethodResponse::SetEmail(mut set_response) => {
                            new_state = set_response.take_new_state();
                            if let Some(updated_ids_) = set_response.take_updated_ids() {
                                updated_ids.extend(updated_ids_);
                            }
                        }
                        MethodResponse::Error(err) => {
                            return Err(jmap_client::Error::from(err)
                                .into_status_response()
                                .with_tag(response.tag.unwrap()));
                        }
                        set_response => {
                            debug!("Received unexpected response from JMAP {:?}", set_response);
                            return Err(StatusResponse::no(
                                "Invalid response received from JMAP server.",
                            )
                            .with_tag(response.tag.unwrap())
                            .with_code(ResponseCode::ContactAdmin));
                        }
                    }
                }

                // Update modseq
                let mut modseq = u32::MAX;
                if is_condstore {
                    if let Some(new_state) = new_state {
                        if let Ok(new_modseq) = self
                            .core
                            .state_to_modseq(&mailbox.account_id, new_state)
                            .await
                        {
                            modseq = new_modseq;
                        }
                    }
                }

                // Verify that all IDs were updated
                if ids.jmap_ids.len() != updated_ids.len() && response.rtype == ResponseType::Ok {
                    response.rtype = ResponseType::No;
                    response.message = if updated_ids.is_empty() {
                        "Operation failed."
                    } else {
                        "Opertation failed for some of the messages."
                    }
                    .into();
                }

                if !emails.is_empty() {
                    // Return flags for all messages.
                    Ok(response.serialize(
                        Response {
                            items: emails
                                .into_iter()
                                .filter_map(|email| {
                                    let mut items = vec![DataItem::Flags {
                                        flags: email
                                            .keywords()
                                            .iter()
                                            .map(|k| Flag::parse_jmap(k.to_string()))
                                            .collect(),
                                    }];
                                    if modseq != u32::MAX
                                        && updated_ids
                                            .iter()
                                            .any(|id| id == email.id().unwrap_or(""))
                                    {
                                        items.push(DataItem::ModSeq { modseq });
                                    }
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
                                        items,
                                    }
                                    .into()
                                })
                                .collect(),
                        }
                        .serialize(),
                    ))
                } else if modseq != u32::MAX && !updated_ids.is_empty() {
                    // If CONDSTORE is enabled, return modseq for updated messages.
                    Ok(response.serialize(
                        Response {
                            items: updated_ids
                                .into_iter()
                                .filter_map(|jmap_id| {
                                    FetchItem {
                                        id: *ids
                                            .jmap_ids
                                            .iter()
                                            .position(|id| id == &jmap_id)
                                            .and_then(|pos| {
                                                if is_uid {
                                                    ids.uids.get(pos)
                                                } else {
                                                    ids.seqnums
                                                        .as_ref()
                                                        .and_then(|ids| ids.get(pos))
                                                }
                                            })?,
                                        items: vec![DataItem::ModSeq { modseq }],
                                    }
                                    .into()
                                })
                                .collect(),
                        }
                        .serialize(),
                    ))
                } else {
                    Err(response)
                }
            }
            Err(err) => Err(err.into_status_response().with_tag(response.tag.unwrap())),
        }
    }
}
