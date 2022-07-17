use jmap_client::mailbox::Role;
use tracing::debug;

use crate::{
    core::{
        client::{Session, SessionData},
        receiver::Request,
        Command, IntoStatusResponse, StatusResponse,
    },
    protocol::{
        list::{
            self, Arguments, Attribute, ChildInfo, ListItem, ReturnOption, SelectionOption, Tag,
        },
        ImapResponse, ProtocolVersion,
    },
};

impl Session {
    pub async fn handle_list(&mut self, request: Request) -> Result<(), ()> {
        match if request.command == Command::List {
            request.parse_list(self.version)
        } else {
            request.parse_lsub()
        } {
            Ok(arguments) => {
                let data = self.state.session_data();
                let version = self.version;
                tokio::spawn(async move {
                    data.list(arguments, version).await;
                });
                Ok(())
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}

impl SessionData {
    pub async fn list(&self, arguments: Arguments, version: ProtocolVersion) {
        let (tag, is_lsub, reference_name, mut patterns, selection_options, return_options) =
            match arguments {
                Arguments::Basic {
                    tag,
                    reference_name,
                    mailbox_name,
                } => (
                    tag,
                    false,
                    reference_name,
                    vec![mailbox_name],
                    Vec::new(),
                    Vec::new(),
                ),
                Arguments::Extended {
                    tag,
                    reference_name,
                    mailbox_name,
                    selection_options,
                    return_options,
                } => (
                    tag,
                    false,
                    reference_name,
                    mailbox_name,
                    selection_options,
                    return_options,
                ),
                Arguments::Lsub {
                    tag,
                    reference_name,
                    mailbox_name,
                } => (
                    tag,
                    true,
                    reference_name,
                    vec![mailbox_name],
                    vec![SelectionOption::Subscribed],
                    Vec::new(),
                ),
            };

        // Refresh mailboxes
        if let Err(err) = self.synchronize_mailboxes(false).await {
            debug!("Failed to refresh mailboxes: {}", err);
            self.write_bytes(err.into_status_response(tag.into()).into_bytes())
                .await;
            return;
        }

        // Process arguments
        let mut filter_subscribed = false;
        let mut recursive_match = false;
        let mut include_subscribed = false;
        let mut include_children = false;
        let mut include_status = None;
        for selection_option in &selection_options {
            match selection_option {
                SelectionOption::Subscribed => {
                    filter_subscribed = true;
                    include_subscribed = true;
                }
                SelectionOption::Remote => (),
                SelectionOption::RecursiveMatch => {
                    recursive_match = true;
                }
            }
        }
        for return_option in &return_options {
            match return_option {
                ReturnOption::Subscribed => {
                    include_subscribed = true;
                }
                ReturnOption::Children => {
                    include_children = true;
                }
                ReturnOption::Status(status) => {
                    include_status = status.into();
                }
            }
        }
        if recursive_match && !filter_subscribed {
            self.write_bytes(
                StatusResponse::bad(
                    tag.into(),
                    None,
                    "RECURSIVEMATCH cannot be the only selection option.",
                )
                .into_bytes(),
            )
            .await;
            return;
        }

        // Append reference name
        if !patterns.is_empty() && !reference_name.is_empty() {
            patterns.iter_mut().for_each(|item| {
                *item = format!("{}{}", reference_name, item);
            })
        }

        let mut list_items = Vec::with_capacity(10);

        // Add "All Mail" folder
        if !filter_subscribed && matches_pattern(&patterns, &self.core.folder_all) {
            list_items.push(ListItem {
                mailbox_name: self.core.folder_all.clone(),
                attributes: vec![Attribute::All, Attribute::NoInferiors],
                tags: vec![],
            });
        }

        // Add mailboxes
        let mut added_shared_folder = false;
        for account in self.mailboxes.lock().iter() {
            if let Some(prefix) = &account.prefix {
                if !added_shared_folder {
                    if !filter_subscribed && matches_pattern(&patterns, &self.core.folder_shared) {
                        list_items.push(ListItem {
                            mailbox_name: self.core.folder_shared.clone(),
                            attributes: if include_children {
                                vec![Attribute::HasChildren, Attribute::NoSelect]
                            } else {
                                vec![Attribute::NoSelect]
                            },
                            tags: vec![],
                        });
                    }
                    added_shared_folder = true;
                }
                if !filter_subscribed && matches_pattern(&patterns, prefix) {
                    list_items.push(ListItem {
                        mailbox_name: prefix.clone(),
                        attributes: if include_children {
                            vec![Attribute::HasChildren, Attribute::NoSelect]
                        } else {
                            vec![Attribute::NoSelect]
                        },
                        tags: vec![],
                    });
                }
            }

            for (mailbox_name, mailbox_id) in &account.mailbox_names {
                if matches_pattern(&patterns, mailbox_name) {
                    let mailbox = account.mailbox_data.get(mailbox_id).unwrap();
                    let mut has_recursive_match = false;
                    if recursive_match {
                        let prefix = format!("{}/", mailbox_name);
                        for (mailbox_name, mailbox_id) in &account.mailbox_names {
                            if mailbox_name.starts_with(&prefix)
                                && account.mailbox_data.get(mailbox_id).unwrap().is_subscribed
                            {
                                has_recursive_match = true;
                                break;
                            }
                        }
                    }
                    if !filter_subscribed || mailbox.is_subscribed || has_recursive_match {
                        let mut attributes = Vec::with_capacity(2);
                        if include_children {
                            attributes.push(if mailbox.has_children {
                                Attribute::HasChildren
                            } else {
                                Attribute::HasNoChildren
                            });
                        }
                        if include_subscribed && mailbox.is_subscribed {
                            attributes.push(Attribute::Subscribed);
                        }
                        match mailbox.role {
                            Role::Archive => attributes.push(Attribute::Archive),
                            Role::Drafts => attributes.push(Attribute::Drafts),
                            Role::Junk => attributes.push(Attribute::Junk),
                            Role::Sent => attributes.push(Attribute::Sent),
                            Role::Trash => attributes.push(Attribute::Trash),
                            _ => (),
                        }
                        list_items.push(ListItem {
                            mailbox_name: mailbox_name.clone(),
                            attributes,
                            tags: if !has_recursive_match {
                                vec![]
                            } else {
                                vec![Tag::ChildInfo(vec![ChildInfo::Subscribed])]
                            },
                        });
                    }
                }
            }
        }

        // Add status response
        let mut status_items = Vec::new();
        if let Some(include_status) = include_status {
            for list_item in &list_items {
                match self
                    .status(list_item.mailbox_name.to_string(), include_status)
                    .await
                {
                    Ok(status) => {
                        status_items.push(status);
                    }
                    Err(err) => {
                        debug!("Failed to get status: {:?}", err);
                    }
                }
            }
        }

        // Write response
        self.write_bytes(
            list::Response {
                is_rev2: version.is_rev2(),
                is_lsub,
                list_items,
                status_items,
            }
            .serialize(tag),
        )
        .await;
    }
}

fn matches_pattern(patterns: &[String], mailbox_name: &str) -> bool {
    if patterns.is_empty() {
        return true;
    }

    for pattern in patterns {
        if pattern == "*" {
            return true;
        } else if pattern == "%" {
            return !mailbox_name.contains('/');
        } else if let Some((prefix, suffix)) = pattern.split_once('*') {
            if (prefix.is_empty() || mailbox_name.starts_with(prefix))
                && (suffix.is_empty() || mailbox_name.ends_with(suffix))
            {
                return true;
            }
        } else if let Some((prefix, suffix)) = pattern.split_once('%') {
            if !prefix.is_empty() {
                if let Some(end) = mailbox_name.strip_prefix(prefix) {
                    if end.contains('/') {
                        continue;
                    }
                } else {
                    continue;
                }
            }
            if suffix.is_empty() || mailbox_name.ends_with(suffix) {
                return true;
            }
        } else if pattern == mailbox_name {
            return true;
        }
    }

    false
}