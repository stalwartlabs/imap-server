use std::{collections::HashMap, sync::Arc};

use jmap_client::{core::response::MethodResponse, email::Property};
use tracing::debug;

use crate::{
    core::{
        client::{SelectedMailbox, Session, SessionData},
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
                let (data, mailbox) = self.state.select_data();
                let is_condstore = self.is_condstore || mailbox.is_condstore;

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
        mailbox: Arc<SelectedMailbox>,
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

        // Convert IMAP ids to JMAP ids.
        let mut ids = match mailbox
            .sequence_to_jmap(&arguments.sequence_set, is_uid)
            .await
        {
            Ok(ids) => {
                if ids.is_empty() {
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
                .modseq_to_state(&mailbox.id.account_id, unchanged_since as u32)
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
            request
                .changes_email(state)
                .account_id(&mailbox.id.account_id);
            match request.send_changes_email().await {
                Ok(changes) => {
                    let mut modified = Vec::new();
                    let mut unchanged_ids = HashMap::with_capacity(ids.len());
                    let mut sequence_set = arguments.sequence_set.try_expand();

                    // Add all IDs that changed in this mailbox
                    for (jmap_id, imap_id) in ids {
                        let was_destroyed = if changes.destroyed().contains(&jmap_id) {
                            unchanged_failed = true;
                            true
                        } else {
                            false
                        };
                        let id = if is_uid { imap_id.uid } else { imap_id.seqnum };
                        if let Some(sequence_set) = sequence_set.as_mut() {
                            sequence_set.remove(&id);
                        }

                        if changes.updated().contains(&jmap_id)
                            || changes.created().contains(&jmap_id)
                            || was_destroyed
                        {
                            modified.push(id);
                        } else {
                            unchanged_ids.insert(jmap_id, imap_id);
                        }
                    }

                    // Add ids that were removed
                    if let Some(sequence_set) = sequence_set {
                        if !sequence_set.is_empty() {
                            modified.extend(sequence_set);
                            unchanged_failed = true;
                        }
                    }

                    ids = unchanged_ids;
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
        if ids.is_empty() {
            return Err(response);
        }

        // Update
        let mut request = self.client.build();
        let ids_vec = ids.keys().collect::<Vec<_>>();
        for jmap_ids_chunk in ids_vec.chunks(max_objects_in_set) {
            let set_request = request.set_email().account_id(&mailbox.id.account_id);
            for &jmap_id in jmap_ids_chunk {
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
            for jmap_ids_chunk in ids_vec.chunks(max_objects_in_get) {
                request
                    .get_email()
                    .account_id(&mailbox.id.account_id)
                    .ids(jmap_ids_chunk.iter().cloned())
                    .properties([Property::Id, Property::Keywords]);
            }
        }

        match request.send().await {
            Ok(set_response) => {
                let mut emails = Vec::new();
                let mut new_state = String::new();
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
                    if let Ok(new_modseq) = self
                        .core
                        .state_to_modseq(&mailbox.id.account_id, new_state.clone())
                        .await
                    {
                        modseq = new_modseq;
                    }
                }
                mailbox.state.lock().last_state = new_state;

                // Verify that all IDs were updated
                if ids.len() != updated_ids.len() && response.rtype == ResponseType::Ok {
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
                                    let imap_id = ids.get(email.id().unwrap_or(""))?;
                                    if is_uid {
                                        items.push(DataItem::Uid { uid: imap_id.uid });
                                    }
                                    if modseq != u32::MAX
                                        && updated_ids
                                            .iter()
                                            .any(|id| id == email.id().unwrap_or(""))
                                    {
                                        items.push(DataItem::ModSeq { modseq });
                                    }
                                    FetchItem {
                                        id: imap_id.seqnum,
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
                                    let imap_id = ids.get(&jmap_id)?;

                                    FetchItem {
                                        id: imap_id.seqnum,
                                        items: if is_uid {
                                            vec![
                                                DataItem::ModSeq { modseq },
                                                DataItem::Uid { uid: imap_id.uid },
                                            ]
                                        } else {
                                            vec![DataItem::ModSeq { modseq }]
                                        },
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
