use std::sync::Arc;

use futures::{Stream, StreamExt};
use jmap_client::{event_source::Changes, TypeState};
use tokio::sync::watch;
use tracing::debug;

use crate::{
    core::{
        client::{Session, SessionData, State},
        message::MailboxData,
        receiver::Request,
        Command, ResponseCode, StatusResponse,
    },
    protocol::{
        list::{Attribute, ListItem},
        status::Status,
    },
};

impl Session {
    pub async fn handle_idle(&mut self, request: Request) -> Result<(), ()> {
        let (data, mailbox, subscriptions) = match &self.state {
            State::Authenticated { data } => (data.clone(), None, vec![TypeState::Mailbox]),
            State::Selected { data, mailbox, .. } => (
                data.clone(),
                mailbox.clone().into(),
                vec![TypeState::Email, TypeState::Mailbox],
            ),
            _ => unreachable!(),
        };

        // Start event source
        let changes = match data
            .client
            .event_source(subscriptions.into(), false, 30.into(), None)
            .await
        {
            Ok(changes) => changes,
            Err(err) => {
                debug!("Error starting event source: {}", err);
                return self
                    .write_bytes(
                        StatusResponse::no("It was not possible to start IDLE.")
                            .with_tag(request.tag)
                            .with_code(ResponseCode::ContactAdmin)
                            .into_bytes(),
                    )
                    .await;
            }
        };

        // Send continuation response
        self.write_bytes(b"+ Idling, send 'DONE' to stop.\r\n".to_vec())
            .await?;

        // Create channel
        let (idle_tx, idle_rx) = watch::channel(true);
        self.idle_tx = idle_tx.into();
        let is_rev2 = self.version.is_rev2();

        tokio::spawn(async move {
            data.idle(mailbox, changes, idle_rx, request.tag, is_rev2)
                .await;
        });
        Ok(())
    }
}

impl SessionData {
    pub async fn idle(
        &self,
        mailbox: Option<Arc<MailboxData>>,
        mut changes: impl Stream<Item = jmap_client::Result<Changes>> + Unpin,
        mut idle_rx: watch::Receiver<bool>,
        tag: String,
        is_rev2: bool,
    ) {
        // Obtain current state
        let mut email_state = if let Some(mailbox) = &mailbox {
            let mut request = self.client.build();
            request
                .get_email()
                .account_id(&mailbox.account_id)
                .ids(Vec::<&str>::new());
            match request.send_get_email().await {
                Ok(mut response) => response.take_state().into(),
                Err(err) => {
                    debug!("Failed to obtain state: {}", err);
                    None
                }
            }
        } else {
            None
        };

        loop {
            tokio::select! {
                changes = changes.next() => {
                    match changes {
                        Some(Ok(changes)) => {
                            let mut has_mailbox_changes = false;
                            let mut new_email_state = None;
                            for (account_id, changes) in changes.into_inner() {
                                for (type_state, change_id) in changes {
                                    match type_state {
                                        TypeState::Mailbox => {
                                            has_mailbox_changes = true;
                                        }
                                        TypeState::Email if mailbox.as_ref().map_or(false, |m| m.account_id == account_id) => {
                                            new_email_state = Some(change_id);
                                        }
                                        _ => (),
                                    }
                                }
                            }

                            let has_email_changes = if new_email_state.is_some() {
                                let tmp_state = email_state;
                                email_state = new_email_state;
                                tmp_state
                            } else {
                                None
                            };

                            self.write_changes(mailbox.as_ref(), has_mailbox_changes, has_email_changes, is_rev2).await;


                        },
                        Some(Err(err)) => {
                            debug!("EventSource error: {}", err);
                        }
                        None => {
                            debug!("EventSource connection unexpectedly closed.");
                            break;
                        },
                    }
                },
                _ = idle_rx.changed() => {
                    self.write_bytes(StatusResponse::completed(Command::Idle).with_tag(tag).into_bytes())
                        .await;
                    return;
                }
            };
        }

        // Connection was unexpectedly closed.
        // TODO: Try reconnecting.
        idle_rx.changed().await.ok();
        self.write_bytes(
            StatusResponse::completed(Command::Idle)
                .with_tag(tag)
                .into_bytes(),
        )
        .await;
    }

    pub async fn write_changes(
        &self,
        mailbox: Option<&Arc<MailboxData>>,
        check_mailboxes: bool,
        check_emails: Option<String>,
        is_rev2: bool,
    ) {
        let mut buf = Vec::with_capacity(64);

        // Fetch all changed mailboxes
        if check_mailboxes {
            match self.synchronize_mailboxes(true, false).await {
                Ok(Some(changes)) => {
                    // List deleted mailboxes
                    for mailbox_name in changes.deleted {
                        ListItem {
                            mailbox_name,
                            attributes: vec![Attribute::NonExistent],
                            tags: vec![],
                        }
                        .serialize(&mut buf, is_rev2, false);
                    }

                    // List added mailboxes
                    for mailbox_name in changes.added {
                        ListItem {
                            mailbox_name: mailbox_name.to_string(),
                            attributes: vec![],
                            tags: vec![],
                        }
                        .serialize(&mut buf, is_rev2, false);
                    }

                    // Obtain status of changed mailboxes
                    for mailbox_name in changes.changed {
                        if let Ok(status) = self
                            .status(
                                mailbox_name,
                                &[
                                    Status::Messages,
                                    Status::Unseen,
                                    Status::UidNext,
                                    Status::UidValidity,
                                ],
                            )
                            .await
                        {
                            status.serialize(&mut buf, is_rev2);
                        }
                    }
                }
                Err(err) => {
                    debug!("Failed to refresh mailboxes: {}", err);
                }
                _ => unreachable!(),
            }
        }

        // Fetch selected mailbox changes
        if let (Some(mailbox), Some(check_emails)) = (mailbox, check_emails) {
            let mut request = self.client.build();
            request
                .changes_email(check_emails)
                .account_id(&mailbox.account_id);
            match request.send_changes_email().await {
                Ok(mut changes) => {
                    // Obtain deleted seqnums
                    let mut has_deletions = false;
                    if !changes.destroyed().is_empty() {
                        if let Ok(ids) = self
                            .core
                            .jmap_deletions_to_imap(
                                mailbox.clone(),
                                changes.take_destroyed(),
                                false,
                                false,
                            )
                            .await
                        {
                            for id in ids {
                                has_deletions = true;
                                buf.extend_from_slice(format!("* {} EXPUNGE\r\n", id).as_bytes());
                            }
                        }
                    }

                    // Synchronize emails
                    if let Ok(mailbox_status) = self.synchronize_messages(mailbox.clone()).await {
                        let mut has_changes = false;
                        if !changes.updated().is_empty() || !changes.created().is_empty() {
                            if let Ok(ids) = self
                                .core
                                .jmap_to_imap(
                                    mailbox.clone(),
                                    changes
                                        .take_created()
                                        .into_iter()
                                        .chain(changes.take_updated())
                                        .collect::<Vec<_>>(),
                                    false,
                                    true,
                                )
                                .await
                            {
                                if !ids.uids.is_empty() {
                                    has_changes = true;
                                }
                            }
                        };

                        if has_changes || has_deletions {
                            buf.extend_from_slice(
                                format!("* {} EXISTS\r\n", mailbox_status.total_messages)
                                    .as_bytes(),
                            );
                        }
                    }
                }
                Err(err) => {
                    debug!("Failed to fetch email changes: {}", err);
                }
            }
        }

        // Write changes
        if !buf.is_empty() {
            self.write_bytes(buf).await;
        }
    }
}
