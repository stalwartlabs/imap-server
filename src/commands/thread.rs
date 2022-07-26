use std::{collections::HashMap, sync::Arc};

use jmap_client::{core::response::MethodResponse, email::Property};
use tracing::debug;

use crate::{
    core::{
        client::{SelectedMailbox, Session, SessionData},
        receiver::Request,
        IntoStatusResponse, StatusResponse,
    },
    protocol::{
        select::Exists,
        thread::{Arguments, Response},
        ImapResponse,
    },
};

impl Session {
    pub async fn handle_thread(&mut self, request: Request, is_uid: bool) -> Result<(), ()> {
        let command = request.command;
        match request.parse_thread() {
            Ok(arguments) => {
                let (data, mailbox) = self.state.mailbox_data();

                tokio::spawn(async move {
                    let bytes = match data.thread(arguments, mailbox, is_uid).await {
                        Ok((response, tag)) => StatusResponse::completed(command)
                            .with_tag(tag)
                            .serialize(response.serialize()),
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
    pub async fn thread(
        &self,
        arguments: Arguments,
        mailbox: Arc<SelectedMailbox>,
        is_uid: bool,
    ) -> Result<(Response, String), StatusResponse> {
        // Convert IMAP to JMAP query
        let (filter, _) = self
            .imap_filter_to_jmap(arguments.filter, mailbox.clone(), None, is_uid)
            .await?;

        // Build query
        let max_objects_in_get = self
            .client
            .session()
            .core_capabilities()
            .map(|c| c.max_objects_in_get())
            .unwrap_or(500);
        let mut position = 0;
        let mut jmap_ids = Vec::new();
        let mut threads = HashMap::new();
        loop {
            let mut total = 0;
            let mut request = self.client.build();
            let query_result = request
                .query_email()
                .filter(filter.clone())
                .calculate_total(true)
                .position(position)
                .limit(max_objects_in_get)
                .result_reference();
            request
                .get_email()
                .ids_ref(query_result)
                .properties([Property::Id, Property::ThreadId]);

            let mut results_len = 0;
            for response in request
                .send()
                .await
                .map_err(|err| {
                    err.into_status_response()
                        .with_tag(arguments.tag.to_string())
                })?
                .unwrap_method_responses()
            {
                match response.unwrap_method_response() {
                    MethodResponse::GetEmail(mut response) => {
                        for mut email in response.take_list() {
                            if let Some(thread_id) = email.take_thread_id() {
                                threads
                                    .entry(thread_id)
                                    .or_insert_with(Vec::new)
                                    .push(email.take_id());
                            }
                        }
                    }
                    MethodResponse::QueryEmail(mut response) => {
                        let results = response.take_ids();
                        total = response.total().unwrap_or(0);
                        results_len = results.len();
                        if results_len > 0 {
                            jmap_ids.extend(results);
                        }
                    }
                    MethodResponse::Error(err) => {
                        return Err(jmap_client::Error::from(err)
                            .into_status_response()
                            .with_tag(arguments.tag));
                    }
                    response => {
                        debug!("Unexpected response: {:?}", response);
                        break;
                    }
                }
            }

            if results_len > 0 && jmap_ids.len() < total {
                position += results_len as i32;
                continue;
            }
            break;
        }

        // Check that the mailbox is in-sync
        if !mailbox.is_in_sync(&jmap_ids) {
            // Mailbox is out of sync
            let new_state = self
                .synchronize_messages(mailbox.id.clone())
                .await
                .map_err(|err| err.with_tag(arguments.tag.to_string()))?;
            let (new_message_count, _) =
                mailbox.synchronize_uids(new_state.jmap_ids, new_state.imap_uids, false);

            if let Some(new_message_count) = new_message_count {
                self.write_bytes(
                    Exists {
                        total_messages: new_message_count,
                    }
                    .into_bytes(),
                )
                .await;
            }
        }

        // Build response
        let threads = threads
            .values()
            .map(|jmap_ids| {
                mailbox
                    .jmap_to_imap(jmap_ids)
                    .into_iter()
                    .map(|id| if is_uid { id.uid } else { id.seqnum })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        // Build response
        Ok((Response { is_uid, threads }, arguments.tag))
    }
}
