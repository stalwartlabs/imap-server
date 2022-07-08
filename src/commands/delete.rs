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
        if let Err(err) = self.synchronize_mailboxes().await {
            debug!("Failed to refresh mailboxes: {}", err);
            return err.into_status_response(arguments.tag.into());
        }

        // Validate mailbox
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
                            mailbox_id =
                                (account.account_id.to_string(), mailbox_id_.to_string()).into();
                            break 'outer;
                        } else if mailbox_name.starts_with(&prefix) {
                            return StatusResponse::no(
                                arguments.tag.into(),
                                ResponseCode::HasChildren.into(),
                                "Mailbox has children that need to be deleted first.",
                            );
                        }
                    }
                }
            }
            if let Some(mailbox_id) = mailbox_id {
                mailbox_id
            } else {
                return StatusResponse::no(arguments.tag.into(), None, "Mailbox does not exist.");
            }
        };

        // Delete mailbox
        if let Err(err) = self.client.mailbox_destroy(&mailbox_id, true).await {
            return err.into_status_response(arguments.tag.into());
        }

        // Update mailbox cache
        for account in self.mailboxes.lock().iter_mut() {
            if account.account_id == account_id {
                account.mailbox_names.remove(&arguments.mailbox_name);
                account.mailbox_data.remove(&mailbox_id);
                break;
            }
        }

        StatusResponse::ok(arguments.tag.into(), None, "Mailbox deleted.")
    }
}
