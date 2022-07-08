use jmap_client::core::set::SetObject;
use tracing::debug;

use crate::{
    core::{
        client::{Session, SessionData},
        receiver::Request,
        IntoStatusResponse, ResponseCode, StatusResponse,
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
        if let Err(err) = self.synchronize_mailboxes().await {
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
                            ResponseCode::Cannot.into(),
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
                    ResponseCode::NonExistent.into(),
                    format!("Mailbox '{}' not found.", arguments.mailbox_name),
                );
            }
        };

        // Get new mailbox name from path
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
                        // Update state
                        if let Some(new_state) = response.unwrap_new_state() {
                            account.state_id = new_state;
                        }

                        // Update parents
                        if arguments.mailbox_name.contains('/') {
                            let mut parent_path =
                                arguments.mailbox_name.split('/').collect::<Vec<_>>();
                            parent_path.pop();
                            let parent_path = parent_path.join("/");
                            if let Some(old_parent_id) = account.mailbox_names.get(&parent_path) {
                                if let Some(old_parent) =
                                    account.mailbox_data.get_mut(old_parent_id)
                                {
                                    let prefix = format!("{}/", parent_path);
                                    old_parent.has_children =
                                        account.mailbox_names.keys().any(|name| {
                                            name != &arguments.mailbox_name
                                                && name.starts_with(&prefix)
                                        });
                                }
                            }
                        }
                        if let Some(parent_mailbox) = params
                            .parent_mailbox_id
                            .and_then(|id| account.mailbox_data.get_mut(&id))
                        {
                            parent_mailbox.has_children = true;
                        }

                        let prefix = format!("{}/", arguments.mailbox_name);
                        let mut new_mailbox_names = BTreeMap::new();
                        for (mailbox_name, mailbox_id) in std::mem::take(&mut account.mailbox_names)
                        {
                            if mailbox_name != arguments.mailbox_name {
                                if let Some(child_name) = mailbox_name.strip_prefix(&prefix) {
                                    new_mailbox_names.insert(
                                        format!("{}/{}", params.full_path, child_name),
                                        mailbox_id,
                                    );
                                } else {
                                    new_mailbox_names.insert(mailbox_name, mailbox_id);
                                }
                            }
                        }
                        new_mailbox_names.insert(params.full_path, mailbox_id);
                        account.mailbox_names = new_mailbox_names;
                        break;
                    }
                }

                StatusResponse::ok(arguments.tag.into(), None, "RENAME completed")
            }
            Err(err) => err.into_status_response(arguments.tag.into()),
        }
    }
}
