use tracing::debug;

use crate::{
    core::{
        client::{Session, SessionData},
        receiver::Request,
        IntoStatusResponse, ResponseCode, StatusResponse,
    },
    protocol::delete::Arguments,
};

impl Session {
    pub async fn handle_delete(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_delete(self.version) {
            Ok(arguments) => {
                let data = self.state.session_data();
                tokio::spawn(async move {
                    data.write_bytes(data.delete_folder(arguments).await.into_bytes())
                        .await;
                });
                Ok(())
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}

impl SessionData {
    pub async fn delete_folder(&self, arguments: Arguments) -> StatusResponse {
        // Refresh mailboxes
        if let Err(err) = self.synchronize_mailboxes(false).await {
            debug!("Failed to refresh mailboxes: {}", err);
            return err.into_status_response().with_tag(arguments.tag);
        }

        // Validate mailbox
        let mut delete_uid_cache = false;
        let (account_id, mailbox_id) = {
            let prefix = format!("{}/", arguments.mailbox_name);
            let mut mailbox_id = None;
            'outer: for account in self.mailboxes.lock().iter() {
                if account
                    .prefix
                    .as_ref()
                    .map_or(true, |p| arguments.mailbox_name.starts_with(p))
                {
                    for (mailbox_name, mailbox_id_) in account.mailbox_names.iter() {
                        if mailbox_name == &arguments.mailbox_name {
                            delete_uid_cache = account.prefix.is_none();
                            mailbox_id =
                                (account.account_id.to_string(), mailbox_id_.to_string()).into();
                            break 'outer;
                        } else if mailbox_name.starts_with(&prefix) {
                            return StatusResponse::no(
                                "Mailbox has children that need to be deleted first.",
                            )
                            .with_tag(arguments.tag)
                            .with_code(ResponseCode::HasChildren);
                        }
                    }
                }
            }
            if let Some(mailbox_id) = mailbox_id {
                mailbox_id
            } else {
                return StatusResponse::no("Mailbox does not exist.").with_tag(arguments.tag);
            }
        };

        // Delete mailbox
        if let Err(err) = self.client.mailbox_destroy(&mailbox_id, true).await {
            return err.into_status_response().with_tag(arguments.tag);
        }

        // Delete UID cache
        if delete_uid_cache {
            self.core
                .delete_mailbox(&account_id, &mailbox_id)
                .await
                .ok();
        }

        // Update mailbox cache
        for account in self.mailboxes.lock().iter_mut() {
            if account.account_id == account_id {
                account.mailbox_names.remove(&arguments.mailbox_name);
                account.mailbox_data.remove(&mailbox_id);
                break;
            }
        }

        StatusResponse::ok("Mailbox deleted.").with_tag(arguments.tag)
    }
}
