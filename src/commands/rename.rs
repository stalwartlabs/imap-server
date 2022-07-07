use jmap_client::core::set::SetObject;
use tracing::debug;

use crate::{
    core::{
        client::{Session, SessionData},
        receiver::Request,
        IntoStatusResponse, StatusResponse,
    },
    protocol::rename::Arguments,
};
use std::collections::BTreeMap;

impl Session {
    pub async fn handle_rename(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_rename(self.version) {
            Ok(arguments) => {
                let data = self.state.session_data();
                tokio::spawn(async move {
                    data.write_bytes(data.rename_folder(arguments).await.into_bytes())
                        .await;
                });
                Ok(())
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}

impl SessionData {
    pub async fn rename_folder(&self, arguments: Arguments) -> StatusResponse {
        // Refresh mailboxes
        if let Err(err) = self.refresh_mailboxes().await {
            debug!("Failed to refresh mailboxes: {}", err);

            return err.into_status_response(arguments.tag.into());
        }

        // Validate mailbox name
        let mut params = match self.validate_mailbox_create(&arguments.new_mailbox_name) {
            Ok(response) => response,
            Err(message) => {
                return StatusResponse::no(arguments.tag.into(), None, message);
            }
        };

        // Validate source mailbox
        let mailbox_id = {
            let mut mailbox_id = None;
            for account in self.mailboxes.lock().iter() {
                if let Some(mailbox_id_) = account.mailbox_names.get(&arguments.mailbox_name) {
                    if account.account_id == params.account_id {
                        mailbox_id = mailbox_id_.to_string().into();
                        break;
                    } else {
                        return StatusResponse::no(
                            arguments.tag.into(),
                            None,
                            "Cannot move mailboxes between accounts.",
                        );
                    }
                }
            }
            if let Some(mailbox_id) = mailbox_id {
                mailbox_id
            } else {
                return StatusResponse::no(
                    arguments.tag.into(),
                    None,
                    format!("Mailbox '{}' not found.", arguments.mailbox_name),
                );
            }
        };

        // Get new mailbox name from path
        let new_mailbox_path = if params.path.len() > 1 {
            params.path.join("/")
        } else {
            params.path.last().unwrap().to_string()
        };
        let new_mailbox_name = params.path.pop().unwrap();

        // Build request
        let mut request = self.client.build();
        let mut create_ids: Vec<String> = Vec::with_capacity(params.path.len());
        let set_request = request.set_mailbox().account_id(&params.account_id);
        for path_item in &params.path {
            let create_item = set_request.create().name(*path_item);
            if let Some(create_id) = create_ids.last() {
                create_item.parent_id_ref(create_id);
            } else {
                create_item.parent_id(params.parent_mailbox_id.as_ref());
            }
            create_ids.push(create_item.create_id().unwrap());
        }
        let update_item = set_request.update(&mailbox_id).name(new_mailbox_name);
        if let Some(create_id) = create_ids.last() {
            update_item.parent_id_ref(create_id);
        } else {
            update_item.parent_id(params.parent_mailbox_id.as_ref());
        }

        match request.send_set_mailbox().await {
            Ok(mut response) => {
                let mut mailboxes = if !create_ids.is_empty() {
                    match self.add_created_mailboxes(&mut params, create_ids, &mut response) {
                        Ok(mailboxes) => mailboxes,
                        Err(message) => {
                            return StatusResponse::no(arguments.tag.into(), None, message);
                        }
                    }
                } else {
                    self.mailboxes.lock()
                };
                if let Err(err) = response.updated(&mailbox_id) {
                    return err.into_status_response(arguments.tag.into());
                }

                // Rename mailbox cache
                for account in mailboxes.iter_mut() {
                    if account.account_id == params.account_id {
                        let prefix = format!("{}/", new_mailbox_path);
                        let mut new_mailbox_names = BTreeMap::new();
                        for (mailbox_name, mailbox_id) in std::mem::take(&mut account.mailbox_names)
                        {
                            if mailbox_name != arguments.mailbox_name {
                                if let Some(child_name) = mailbox_name.strip_prefix(&prefix) {
                                    new_mailbox_names
                                        .insert(format!("{}{}", prefix, child_name), mailbox_id);
                                } else {
                                    new_mailbox_names.insert(mailbox_name, mailbox_id);
                                }
                            }
                        }
                        new_mailbox_names.insert(new_mailbox_path, mailbox_id);
                        account.mailbox_names = new_mailbox_names;
                        break;
                    }
                }

                StatusResponse::ok(arguments.tag.into(), None, "Mailbox renamed.")
            }
            Err(err) => err.into_status_response(arguments.tag.into()),
        }
    }
}
