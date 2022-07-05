use std::sync::Arc;

use jmap_client::{
    client::Client,
    mailbox::{Property, Role},
};
use tracing::debug;

use crate::{
    core::{
        client::{Session, SessionData},
        receiver::Request,
        Command, StatusResponse,
    },
    parser::{list::parse_list, lsub::parse_lsub},
    protocol::{
        list::{self, Arguments, Attribute, ListItem},
        lsub,
        status::Status,
        ImapResponse, ProtocolVersion,
    },
};

impl Session {
    pub async fn handle_list(&mut self, request: Request) -> Result<(), ()> {
        match if request.command == Command::List {
            parse_list(request)
        } else {
            parse_lsub(request)
        } {
            Ok(arguments) => {
                spawn_list(self.state.session_data(), self.version, arguments);
                Ok(())
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}

fn spawn_list(data: Arc<SessionData>, version: ProtocolVersion, arguments: Arguments) {
    tokio::spawn(async move {
        let (tag, is_lsub) = match arguments {
            Arguments::Basic {
                tag,
                reference_name,
                mailbox_name,
            } => (tag, false),
            Arguments::Extended {
                tag,
                reference_name,
                mailbox_name,
                selection_options,
                return_options,
            } => (tag, false),
            Arguments::Lsub {
                tag,
                reference_name,
                mailbox_name,
            } => (tag, true),
        };

        let mut list_items = vec![ListItem {
            mailbox_name: data.config.folder_all.clone(),
            attributes: vec![Attribute::All],
            tags: vec![],
        }];

        // Fetch mailboxes for the main account
        if let Err(err) = fetch_mailboxes(&data.client, None, None, &mut list_items, &[]).await {
            debug!("Failed to fetch mailboxes: {}", err);
            data.write_bytes(
                StatusResponse::no(tag.into(), None, "Failed to fetch mailboxes").into_bytes(),
            )
            .await;
            return;
        }

        // Fetch shared mailboxes
        let session = data.client.session();
        for account_id in session.accounts() {
            if account_id != data.client.default_account_id() {
                if let Err(err) = fetch_mailboxes(
                    &data.client,
                    Some(account_id),
                    format!(
                        "{}/{}",
                        data.config.folder_shared,
                        session.account(account_id).unwrap().name()
                    )
                    .into(),
                    &mut list_items,
                    &[],
                )
                .await
                {
                    debug!(
                        "Failed to fetch mailboxes for account {}: {}",
                        account_id, err
                    );
                }
            }
        }

        // Write response
        data.write_bytes(if !is_lsub {
            list::Response {
                list_items,
                status_items: vec![],
            }
            .serialize(tag, version)
        } else {
            lsub::Response { list_items }.serialize(tag, version)
        })
        .await;
    });
}

async fn fetch_mailboxes(
    client: &Client,
    account_id: Option<&str>,
    mailbox_prefix: Option<String>,
    list_items: &mut Vec<ListItem>,
    status: &[Status],
) -> jmap_client::Result<()> {
    let max_objects_in_get = client
        .session()
        .core_capabilities()
        .map(|c| c.max_objects_in_get())
        .unwrap_or(100);
    let mut position = 0;
    let mut mailboxes = Vec::with_capacity(10);
    let mut properties = vec![
        Property::Id,
        Property::Name,
        Property::IsSubscribed,
        Property::ParentId,
        Property::Role,
    ];
    if status.contains(&Status::Messages) {
        properties.push(Property::TotalEmails);
    }
    if status.contains(&Status::Unseen) {
        properties.push(Property::UnreadEmails);
    }

    for _ in 0..100 {
        let mut request = client.build();
        if let Some(account_id) = account_id {
            request = request.account_id(account_id);
        }
        let query_result = request
            .query_mailbox()
            .calculate_total(true)
            .position(position)
            .limit(max_objects_in_get)
            .result_reference();
        request
            .get_mailbox()
            .ids_ref(query_result)
            .properties(properties.clone());

        let mut response = request.send().await?.unwrap_method_responses();
        if response.len() != 2 {
            return Err(jmap_client::Error::Internal(
                "Invalid response while fetching mailboxes".to_string(),
            ));
        }
        let mailboxes_part = response.pop().unwrap().unwrap_get_mailbox()?.unwrap_list();
        let total_mailboxes = response
            .pop()
            .unwrap()
            .unwrap_query_mailbox()?
            .total()
            .unwrap_or(0);

        let mailboxes_part_len = mailboxes_part.len();
        if mailboxes_part_len > 0 {
            mailboxes.extend(mailboxes_part);
            if mailboxes.len() < total_mailboxes {
                position += mailboxes_part_len as i32;
                continue;
            }
        }
        break;
    }

    let mut iter = mailboxes.iter();
    let mut parent_id = None;
    let mut path = Vec::new();
    let mut iter_stack = Vec::new();

    if let Some(mailbox_prefix) = mailbox_prefix {
        path.push(mailbox_prefix);
    }

    // Build list item tree
    loop {
        while let Some(mailbox) = iter.next() {
            if mailbox.parent_id() == parent_id {
                let mut mailbox_path = path.clone();
                let mailbox_role = mailbox.role();
                if mailbox_role != Role::Inbox {
                    mailbox_path.push(mailbox.name().map(|n| n.to_string()).unwrap_or_default());
                } else {
                    mailbox_path.push("INBOX".to_string());
                }
                let has_children = mailboxes
                    .iter()
                    .any(|child| child.parent_id() == Some(mailbox.id()));
                let mut attributes = Vec::new();
                match mailbox_role {
                    Role::Archive => attributes.push(Attribute::Archive),
                    Role::Drafts => attributes.push(Attribute::Drafts),
                    Role::Junk => attributes.push(Attribute::Junk),
                    Role::Sent => attributes.push(Attribute::Sent),
                    Role::Trash => attributes.push(Attribute::Trash),
                    _ => (),
                }
                if mailbox.is_subscribed() {
                    attributes.push(Attribute::Subscribed);
                }
                attributes.push(if has_children {
                    Attribute::HasChildren
                } else {
                    Attribute::HasNoChildren
                });
                list_items.push(ListItem {
                    mailbox_name: mailbox_path.join("/"),
                    attributes,
                    tags: vec![],
                });

                if has_children && iter_stack.len() < 100 {
                    iter_stack.push((iter, parent_id, path));
                    parent_id = Some(mailbox.id());
                    path = mailbox_path;
                    iter = mailboxes.iter();
                }
            }
        }

        if let Some((prev_iter, prev_parent_id, prev_path)) = iter_stack.pop() {
            iter = prev_iter;
            parent_id = prev_parent_id;
            path = prev_path;
        } else {
            break;
        }
    }

    Ok(())
}
