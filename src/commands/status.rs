use std::sync::Arc;

use jmap_client::{
    core::query,
    email::{query::Filter, Property},
};
use tracing::debug;

use crate::{
    core::{
        client::{Session, SessionData},
        mailbox::Mailbox,
        receiver::Request,
        Flag, IntoStatusResponse, ResponseCode, StatusResponse,
    },
    protocol::{
        status::{Response, Status, StatusItem},
        ImapResponse,
    },
};

impl Session {
    pub async fn handle_status(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_status(self.version) {
            Ok(arguments) => {
                let version = self.version;
                let data = self.state.session_data();
                tokio::spawn(async move {
                    // Refresh mailboxes
                    if let Err(err) = data.synchronize_mailboxes().await {
                        debug!("Failed to refresh mailboxes: {}", err);
                        data.write_bytes(
                            err.into_status_response(arguments.tag.into()).into_bytes(),
                        )
                        .await;
                        return;
                    }

                    // Fetch status
                    match data.status(arguments.mailbox_name, &arguments.items).await {
                        Ok(status) => {
                            data.write_bytes(Response { status }.serialize(arguments.tag, version))
                                .await;
                        }
                        Err(mut response) => {
                            response.tag = arguments.tag.into();
                            data.write_bytes(response.into_bytes()).await;
                        }
                    }
                });
                Ok(())
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}

impl SessionData {
    pub async fn status(
        &self,
        mailbox_name: String,
        items: &[Status],
    ) -> crate::core::Result<StatusItem> {
        // Get mailbox id
        let mailbox = if let Some(mailbox) = self.get_mailbox_by_name(&mailbox_name) {
            Arc::new(mailbox)
        } else {
            return Err(StatusResponse::no(
                None,
                ResponseCode::NonExistent.into(),
                "Mailbox does not exist.",
            ));
        };

        // Make sure all requested fields are up to date
        let mut items_update = Vec::with_capacity(items.len());
        let mut items_response = Vec::with_capacity(items.len());

        for account in self.mailboxes.lock().iter_mut() {
            if account.account_id == mailbox.account_id {
                let mailbox_data = account
                    .mailbox_data
                    .entry(
                        mailbox
                            .mailbox_id
                            .as_ref()
                            .cloned()
                            .unwrap_or_else(|| "".to_string()),
                    )
                    .or_insert_with(Mailbox::default);
                for item in items {
                    match item {
                        Status::Messages => {
                            if let Some(value) = mailbox_data.total_messages {
                                items_response.push((*item, value as u32));
                            } else {
                                items_update.push(*item);
                            }
                        }
                        Status::UidNext => {
                            if let Some(value) = mailbox_data.uid_next {
                                items_response.push((*item, value as u32));
                            } else {
                                items_update.push(*item);
                            }
                        }
                        Status::UidValidity => {
                            if let Some(value) = mailbox_data.uid_validity {
                                items_response.push((*item, value as u32));
                            } else {
                                items_update.push(*item);
                            }
                        }
                        Status::Unseen => {
                            if let Some(value) = mailbox_data.total_unseen {
                                items_response.push((*item, value as u32));
                            } else {
                                items_update.push(*item);
                            }
                        }
                        Status::Deleted => {
                            if let Some(value) = mailbox_data.total_deleted {
                                items_response.push((*item, value as u32));
                            } else {
                                items_update.push(*item);
                            }
                        }
                        Status::Size => {
                            if let Some(value) = mailbox_data.size {
                                items_response.push((*item, value as u32));
                            } else {
                                items_update.push(*item);
                            }
                        }
                    }
                }
                break;
            }
        }

        // Update UIDNEXT, UIDVALIDITY and Messages
        if items_update.contains(&Status::UidNext)
            || items_update.contains(&Status::UidValidity)
            || items_update.contains(&Status::Messages)
        {
            let status = self.synchronize_messages(mailbox.clone()).await?;
            for account in self.mailboxes.lock().iter_mut() {
                if account.account_id == mailbox.account_id {
                    let mailbox_data = account
                        .mailbox_data
                        .entry(
                            mailbox
                                .mailbox_id
                                .as_ref()
                                .cloned()
                                .unwrap_or_else(|| "".to_string()),
                        )
                        .or_insert_with(Mailbox::default);
                    mailbox_data.total_messages = status.total_messages.into();
                    mailbox_data.uid_next = status.uid_next.into();
                    mailbox_data.uid_validity = status.uid_validity.into();
                    if items_update.contains(&Status::UidNext) {
                        items_response.push((Status::UidNext, status.uid_next as u32));
                    }
                    if items_update.contains(&Status::UidValidity) {
                        items_response.push((Status::UidValidity, status.uid_validity as u32));
                    }
                    if items_update.contains(&Status::Messages) {
                        items_response.push((Status::Messages, status.total_messages as u32));
                    }
                    break;
                }
            }
        }

        // Update Unseen
        if items_update.contains(&Status::Unseen) || items_update.contains(&Status::Deleted) {
            let mut request = self.client.build();
            if items_update.contains(&Status::Unseen) {
                request
                    .query_email()
                    .account_id(&mailbox.account_id)
                    .filter(query::Filter::and(
                        if let Some(mailbox_id) = &mailbox.mailbox_id {
                            vec![
                                Filter::in_mailbox(mailbox_id),
                                Filter::not_keyword(Flag::Seen.to_jmap()),
                            ]
                        } else {
                            vec![Filter::not_keyword(Flag::Seen.to_jmap())]
                        },
                    ))
                    .calculate_total(true)
                    .limit(1);
            }
            if items_update.contains(&Status::Deleted) {
                request
                    .query_email()
                    .account_id(&mailbox.account_id)
                    .filter(query::Filter::and(
                        if let Some(mailbox_id) = &mailbox.mailbox_id {
                            vec![
                                Filter::in_mailbox(mailbox_id),
                                Filter::has_keyword(Flag::Deleted.to_jmap()),
                            ]
                        } else {
                            vec![Filter::has_keyword(Flag::Deleted.to_jmap())]
                        },
                    ))
                    .calculate_total(true)
                    .limit(1);
            }
            let mut responses = request
                .send()
                .await
                .map_err(|err| err.into_status_response(None))?
                .unwrap_method_responses()
                .into_iter();

            // Update cache
            for account in self.mailboxes.lock().iter_mut() {
                if account.account_id == mailbox.account_id {
                    let mailbox_data = account
                        .mailbox_data
                        .entry(
                            mailbox
                                .mailbox_id
                                .as_ref()
                                .cloned()
                                .unwrap_or_else(|| "".to_string()),
                        )
                        .or_insert_with(Mailbox::default);

                    if items_update.contains(&Status::Unseen) {
                        mailbox_data.total_unseen = responses
                            .next()
                            .ok_or_else(|| {
                                StatusResponse::no(
                                    None,
                                    ResponseCode::ContactAdmin.into(),
                                    "Invalid JMAP server response",
                                )
                            })?
                            .unwrap_query_email()
                            .map_err(|err| err.into_status_response(None))?
                            .total()
                            .unwrap_or(0)
                            .into();
                        items_response
                            .push((Status::Unseen, mailbox_data.total_unseen.unwrap() as u32));
                    }
                    if items_update.contains(&Status::Deleted) {
                        mailbox_data.total_deleted = responses
                            .next()
                            .ok_or_else(|| {
                                StatusResponse::no(
                                    None,
                                    ResponseCode::ContactAdmin.into(),
                                    "Invalid JMAP server response",
                                )
                            })?
                            .unwrap_query_email()
                            .map_err(|err| err.into_status_response(None))?
                            .total()
                            .unwrap_or(0)
                            .into();
                        items_response
                            .push((Status::Unseen, mailbox_data.total_deleted.unwrap() as u32));
                    }
                    break;
                }
            }
        }

        // Update Size
        if items_update.contains(&Status::Size) {
            let max_objects_in_get = self
                .client
                .session()
                .core_capabilities()
                .map(|c| c.max_objects_in_get())
                .unwrap_or(500);
            let mut position = 0;
            let mut mailbox_size = 0;

            // Fetch email sizes
            for _ in 0..100 {
                let mut request = self.client.build().account_id(&mailbox.account_id);
                let query_request = request
                    .query_email()
                    .calculate_total(true)
                    .position(position as i32)
                    .limit(max_objects_in_get);
                if let Some(mailbox_id) = &mailbox.mailbox_id {
                    query_request.filter(Filter::in_mailbox(mailbox_id));
                }

                let query_reference = query_request.result_reference();
                request
                    .get_email()
                    .ids_ref(query_reference)
                    .properties([Property::Size]);

                let mut response = request
                    .send()
                    .await
                    .map_err(|err| err.into_status_response(None))?
                    .unwrap_method_responses();

                if response.len() != 2 {
                    return Err(StatusResponse::no(
                        None,
                        ResponseCode::ContactAdmin.into(),
                        "Invalid JMAP server response.",
                    ));
                }

                let emails = response
                    .pop()
                    .unwrap()
                    .unwrap_get_email()
                    .map_err(|err| err.into_status_response(None))?
                    .take_list();
                if !emails.is_empty() {
                    let total_emails = response
                        .pop()
                        .unwrap()
                        .unwrap_query_email()
                        .map_err(|err| err.into_status_response(None))?
                        .total()
                        .unwrap_or(0);
                    position += emails.len();
                    for email in emails {
                        mailbox_size += email.size();
                    }
                    if position < total_emails {
                        continue;
                    }
                }
                break;
            }

            // Update cache
            for account in self.mailboxes.lock().iter_mut() {
                if account.account_id == mailbox.account_id {
                    account
                        .mailbox_data
                        .entry(
                            mailbox
                                .mailbox_id
                                .as_ref()
                                .cloned()
                                .unwrap_or_else(|| "".to_string()),
                        )
                        .or_insert_with(Mailbox::default)
                        .size = mailbox_size.into();
                    items_response.push((Status::Unseen, mailbox_size as u32));
                    break;
                }
            }
        }

        // Generate response
        Ok(StatusItem {
            mailbox_name,
            items: items_response,
        })
    }
}
