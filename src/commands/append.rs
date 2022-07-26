use std::sync::Arc;

use jmap_client::client::Client;
use tracing::debug;

use crate::{
    core::{
        client::Session, message::MappingOptions, receiver::Request, Command, IntoStatusResponse,
        ResponseCode, StatusResponse,
    },
    protocol::select::Exists,
};

impl Session {
    pub async fn handle_append(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_append() {
            Ok(arguments) => {
                let (data, selected_mailbox) = self.state.session_mailbox_data();

                // Refresh mailboxes
                if let Err(err) = data.synchronize_mailboxes(false, false).await {
                    debug!("Failed to refresh mailboxes: {}", err);
                    return self
                        .write_bytes(
                            err.into_status_response()
                                .with_tag(arguments.tag)
                                .into_bytes(),
                        )
                        .await;
                }

                // Obtain mailbox
                let mailbox =
                    if let Some(mailbox) = data.get_mailbox_by_name(&arguments.mailbox_name) {
                        if mailbox.mailbox_id.is_some() {
                            Arc::new(mailbox)
                        } else {
                            return self
                                .write_bytes(
                                    StatusResponse::no(
                                        "Appending messages to this mailbox is not allowed.",
                                    )
                                    .with_tag(arguments.tag)
                                    .with_code(ResponseCode::Cannot)
                                    .into_bytes(),
                                )
                                .await;
                        }
                    } else {
                        return self
                            .write_bytes(
                                StatusResponse::no("Mailbox does not exist.")
                                    .with_tag(arguments.tag)
                                    .with_code(ResponseCode::TryCreate)
                                    .into_bytes(),
                            )
                            .await;
                    };

                // Check if mailbox is selected
                let is_dest_selected = matches!(&selected_mailbox, Some(selected_mailbox)
                                if selected_mailbox.id.as_ref() == mailbox.as_ref());

                tokio::spawn(async move {
                    let mut created_jmap_ids = Vec::with_capacity(arguments.messages.len());
                    let mut response =
                        StatusResponse::completed(Command::Append).with_tag(arguments.tag);

                    for message in arguments.messages {
                        match append_message(
                            &data.client,
                            &mailbox.account_id,
                            message.message,
                            [mailbox.mailbox_id.as_ref().unwrap()],
                            message.flags.iter().map(|f| f.to_jmap()).into(),
                            message.received_at,
                        )
                        .await
                        {
                            Ok((mut email, new_state)) => {
                                // Update last known state for the selected mailbox
                                if is_dest_selected {
                                    selected_mailbox.as_ref().unwrap().state.lock().last_state =
                                        new_state;
                                }

                                let jmap_id = email.take_id();
                                if !jmap_id.is_empty() {
                                    created_jmap_ids.push(jmap_id);
                                }
                            }
                            Err(err) => {
                                response =
                                    err.into_status_response().with_tag(response.tag.unwrap());
                                break;
                            }
                        }
                    }

                    if !created_jmap_ids.is_empty() {
                        let uids = if let Ok((_, uids)) = data
                            .core
                            .jmap_to_imap(
                                mailbox.clone(),
                                created_jmap_ids,
                                MappingOptions::AddIfMissing,
                            )
                            .await
                        {
                            uids
                        } else {
                            data.write_bytes(
                                StatusResponse::database_failure()
                                    .with_tag(response.tag.unwrap())
                                    .into_bytes(),
                            )
                            .await;
                            return;
                        };

                        // Message was appended to the current mailbox, obtain total count
                        let uid_validity = match selected_mailbox {
                            Some(selected_mailbox)
                                if selected_mailbox.id.as_ref() == mailbox.as_ref() =>
                            {
                                // Mailbox is out of sync
                                let new_state = match data
                                    .synchronize_messages(selected_mailbox.id.clone())
                                    .await
                                {
                                    Ok(new_state) => new_state,
                                    Err(err) => {
                                        data.write_bytes(
                                            err.with_tag(response.tag.unwrap()).into_bytes(),
                                        )
                                        .await;
                                        return;
                                    }
                                };
                                let (new_message_count, _) = selected_mailbox.synchronize_uids(
                                    new_state.jmap_ids,
                                    new_state.imap_uids,
                                    false,
                                );

                                if let Some(new_message_count) = new_message_count {
                                    data.write_bytes(
                                        Exists {
                                            total_messages: new_message_count,
                                        }
                                        .into_bytes(),
                                    )
                                    .await;
                                }

                                new_state.uid_validity
                            }
                            _ => {
                                if let Ok((uid_validity, _)) = data.core.uids(mailbox.clone()).await
                                {
                                    uid_validity
                                } else {
                                    data.write_bytes(
                                        StatusResponse::database_failure()
                                            .with_tag(response.tag.unwrap())
                                            .into_bytes(),
                                    )
                                    .await;
                                    return;
                                }
                            }
                        };

                        response =
                            response.with_code(ResponseCode::AppendUid { uid_validity, uids });
                    }
                    data.write_bytes(response.into_bytes()).await;
                });
                Ok(())
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}

async fn append_message<T, U, V, W>(
    client: &Client,
    account_id: &str,
    raw_message: Vec<u8>,
    mailbox_ids: T,
    keywords: Option<V>,
    received_at: Option<i64>,
) -> jmap_client::Result<(jmap_client::email::Email, String)>
where
    T: IntoIterator<Item = U>,
    U: Into<String>,
    V: IntoIterator<Item = W>,
    W: Into<String>,
{
    let blob_id = client.upload(None, raw_message, None).await?.take_blob_id();
    let mut request = client.build();
    let import_request = request
        .import_email()
        .account_id(account_id)
        .email(blob_id)
        .mailbox_ids(mailbox_ids);

    if let Some(keywords) = keywords {
        import_request.keywords(keywords);
    }

    if let Some(received_at) = received_at {
        import_request.received_at(received_at);
    }

    let id = import_request.create_id();
    let mut response = request
        .send_single::<jmap_client::email::import::EmailImportResponse>()
        .await?;

    Ok((response.created(&id)?, response.take_new_state()))
}
