use std::sync::Arc;

use jmap_client::core::response::MethodResponse;

use crate::{
    core::{
        client::{Session, SessionData},
        message::MailboxData,
        receiver::Request,
        Command, IntoStatusResponse, ResponseCode, StatusResponse,
    },
    protocol::copy_move::Arguments,
};

impl Session {
    pub async fn handle_copy_move(
        &mut self,
        request: Request,
        is_move: bool,
        is_uid: bool,
    ) -> Result<(), ()> {
        match request.parse_copy_move() {
            Ok(arguments) => {
                let (data, src_mailbox, _) = self.state.mailbox_data();

                // Make sure the mailbox exists.
                let dest_mailbox =
                    if let Some(mailbox) = data.get_mailbox_by_name(&arguments.mailbox_name) {
                        if mailbox.mailbox_id.is_some() {
                            Arc::new(mailbox)
                        } else {
                            return self
                                .write_bytes(
                                    StatusResponse::no(
                                        arguments.tag.into(),
                                        ResponseCode::NoPerm.into(),
                                        "Appending messages to this mailbox is not allowed.",
                                    )
                                    .into_bytes(),
                                )
                                .await;
                        }
                    } else {
                        return self
                            .write_bytes(
                                StatusResponse::no(
                                    arguments.tag.into(),
                                    ResponseCode::TryCreate.into(),
                                    "Destination mailbox does not exist.",
                                )
                                .into_bytes(),
                            )
                            .await;
                    };

                // Check that the destination mailbox is not the same as the source mailbox.
                if src_mailbox.account_id == dest_mailbox.account_id
                    && src_mailbox.mailbox_id == dest_mailbox.mailbox_id
                {
                    return self
                        .write_bytes(
                            StatusResponse::no(
                                arguments.tag.into(),
                                ResponseCode::Cannot.into(),
                                "Source and destination mailboxes are the same.",
                            )
                            .into_bytes(),
                        )
                        .await;
                }

                tokio::spawn(async move {
                    if let Err(err) = data
                        .copy_move(arguments, src_mailbox, dest_mailbox, is_move, is_uid)
                        .await
                    {
                        data.write_bytes(err.into_bytes()).await;
                    }
                });
                Ok(())
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}

impl SessionData {
    pub async fn copy_move(
        &self,
        arguments: Arguments,
        src_mailbox: Arc<MailboxData>,
        dest_mailbox: Arc<MailboxData>,
        is_move: bool,
        is_uid: bool,
    ) -> Result<(), StatusResponse> {
        // Convert IMAP ids to JMAP ids.
        let ids = match self
            .imap_sequence_to_jmap(src_mailbox.clone(), arguments.sequence_set, is_uid)
            .await
        {
            Ok(ids) => {
                if ids.uids.is_empty() {
                    return Err(StatusResponse::completed(
                        if is_move {
                            Command::Move(is_uid)
                        } else {
                            Command::Copy(is_uid)
                        },
                        arguments.tag,
                    ));
                }
                ids
            }
            Err(response) => {
                return Err(response.with_tag(arguments.tag));
            }
        };

        let max_objects_in_set = self
            .client
            .session()
            .core_capabilities()
            .map(|c| c.max_objects_in_set())
            .unwrap_or(500);

        let copied_ids = if src_mailbox.account_id == dest_mailbox.account_id {
            // Mailboxes are in the same account, send a Email/set request.
            let mut request = self.client.build();
            for jmap_ids in ids.jmap_ids.chunks(max_objects_in_set) {
                let set_request = request.set_email().account_id(&src_mailbox.account_id);
                for jmap_id in jmap_ids {
                    let update_item = set_request.update(jmap_id);
                    update_item.mailbox_id(dest_mailbox.mailbox_id.as_ref().unwrap(), true);
                    if is_move {
                        if let Some(mailbox_id) = &src_mailbox.mailbox_id {
                            update_item.mailbox_id(mailbox_id, false);
                        }
                    }
                }
            }
            let mut copied_ids = Vec::with_capacity(ids.jmap_ids.len());
            for response in request
                .send()
                .await
                .map_err(|err| err.into_status_response(arguments.tag.to_string().into()))?
                .unwrap_method_responses()
            {
                if let Some(updated_ids) = response
                    .unwrap_set_email()
                    .map_err(|err| err.into_status_response(arguments.tag.to_string().into()))?
                    .take_updated_ids()
                {
                    copied_ids.extend(updated_ids);
                }
            }
            copied_ids
        } else {
            // Mailboxes are in different accounts, send a Email/copy request.
            let mut request = self.client.build();

            for jmap_ids in ids.jmap_ids.chunks(max_objects_in_set) {
                let copy_request = request
                    .copy_email(&src_mailbox.account_id)
                    .account_id(&dest_mailbox.account_id)
                    .on_success_destroy_original(is_move);
                for jmap_id in jmap_ids {
                    copy_request
                        .create(jmap_id)
                        .mailbox_id(dest_mailbox.mailbox_id.as_ref().unwrap(), true);
                }
            }
            let mut copied_ids = Vec::with_capacity(ids.jmap_ids.len());
            for response in request
                .send()
                .await
                .map_err(|err| err.into_status_response(arguments.tag.to_string().into()))?
                .unwrap_method_responses()
            {
                match response.unwrap_method_response() {
                    MethodResponse::CopyEmail(mut response) => {
                        if let Some(updated_ids) = response.take_created() {
                            copied_ids.extend(updated_ids.into_iter().map(|mut m| m.take_id()));
                        }
                    }
                    MethodResponse::Error(err) => {
                        return Err(jmap_client::Error::from(err)
                            .into_status_response(arguments.tag.into()));
                    }
                    _ => (),
                }
            }
            copied_ids
        };

        let ids = self
            .core
            .jmap_to_imap(dest_mailbox.clone(), copied_ids, true, true)
            .await
            .map_err(|_| StatusResponse::database_failure(arguments.tag.to_string().into()))?;
        let (uid_validity, _) = self
            .core
            .uids(dest_mailbox)
            .await
            .map_err(|_| StatusResponse::database_failure(arguments.tag.to_string().into()))?;
        let uid_copy = ResponseCode::CopyUid {
            uid_validity,
            uids: ids.uids,
        };

        if is_move {
            let mut buf = StatusResponse::ok(None, uid_copy.into(), "").into_bytes();
            StatusResponse::completed(Command::Move(is_uid), arguments.tag).serialize(&mut buf);
            self.write_bytes(buf).await;
        } else {
            self.write_bytes(
                StatusResponse::completed(Command::Copy(is_uid), arguments.tag)
                    .with_code(uid_copy)
                    .into_bytes(),
            )
            .await;
        }

        Ok(())
    }
}