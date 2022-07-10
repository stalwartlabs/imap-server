use super::{client::SessionData, message::MailboxData};
use jmap_client::{
    client::Client,
    mailbox::{Property, Role},
};
use std::collections::{BTreeMap, HashMap};
use tracing::debug;

#[derive(Debug, Default)]
pub struct Mailbox {
    pub has_children: bool,
    pub is_subscribed: bool,
    pub role: Role,
    pub total_messages: Option<usize>,
    pub total_unseen: Option<usize>,
    pub total_deleted: Option<usize>,
    pub uid_validity: Option<u32>,
    pub uid_next: Option<u32>,
    pub size: Option<usize>,
}

#[derive(Debug)]
pub struct Account {
    pub account_id: String,
    pub state_id: String,
    pub prefix: Option<String>,
    pub mailbox_names: BTreeMap<String, String>,
    pub mailbox_data: HashMap<String, Mailbox>,
}

pub async fn fetch_mailboxes(client: &Client, folder_shared: &str) -> Option<Vec<Account>> {
    let mut mailboxes = Vec::new();

    // Fetch mailboxes for the main account
    match fetch_account_mailboxes(client, client.default_account_id().to_string(), None).await {
        Ok(account_mailboxes) => {
            mailboxes.push(account_mailboxes);
        }
        Err(err) => {
            debug!("Failed to fetch mailboxes: {}", err);
            return None;
        }
    }

    // Fetch shared mailboxes
    let session = client.session();
    for account_id in session.accounts() {
        if account_id != client.default_account_id() {
            match fetch_account_mailboxes(
                client,
                account_id.to_string(),
                format!(
                    "{}/{}",
                    folder_shared,
                    session.account(account_id).unwrap().name()
                )
                .into(),
            )
            .await
            {
                Ok(account_mailboxes) => {
                    mailboxes.push(account_mailboxes);
                }
                Err(err) => {
                    debug!(
                        "Failed to fetch mailboxes for account {}: {}",
                        account_id, err
                    );
                }
            }
        }
    }

    Some(mailboxes)
}

async fn fetch_account_mailboxes(
    client: &Client,
    account_id: String,
    mailbox_prefix: Option<String>,
) -> jmap_client::Result<Account> {
    let max_objects_in_get = client
        .session()
        .core_capabilities()
        .map(|c| c.max_objects_in_get())
        .unwrap_or(100);
    let mut position = 0;
    let mut result = Vec::with_capacity(10);
    let mut state_id = String::new();

    for _ in 0..100 {
        let mut request = client.build().account_id(&account_id);
        let query_result = request
            .query_mailbox()
            .calculate_total(true)
            .position(position)
            .limit(max_objects_in_get)
            .result_reference();
        request.get_mailbox().ids_ref(query_result).properties([
            Property::Id,
            Property::Name,
            Property::IsSubscribed,
            Property::ParentId,
            Property::Role,
            Property::TotalEmails,
            Property::UnreadEmails,
        ]);

        let mut response = request.send().await?.unwrap_method_responses();
        if response.len() != 2 {
            return Err(jmap_client::Error::Internal(
                "Invalid response while fetching mailboxes".to_string(),
            ));
        }
        let mut get_response = response.pop().unwrap().unwrap_get_mailbox()?;
        state_id = get_response.unwrap_state();
        let mailboxes_part = get_response.unwrap_list();
        let total_mailboxes = response
            .pop()
            .unwrap()
            .unwrap_query_mailbox()?
            .total()
            .unwrap_or(0);

        let mailboxes_part_len = mailboxes_part.len();
        if mailboxes_part_len > 0 {
            result.extend(mailboxes_part);
            if result.len() < total_mailboxes {
                position += mailboxes_part_len as i32;
                continue;
            }
        }
        break;
    }

    let mut iter = result.iter();
    let mut parent_id = None;
    let mut path = Vec::new();
    let mut iter_stack = Vec::new();

    if let Some(mailbox_prefix) = &mailbox_prefix {
        path.push(mailbox_prefix.to_string());
    };

    let mut account = Account {
        account_id,
        state_id,
        prefix: mailbox_prefix,
        mailbox_names: BTreeMap::new(),
        mailbox_data: HashMap::with_capacity(result.len()),
    };

    // Build list item tree
    loop {
        while let Some(mailbox) = iter.next() {
            if mailbox.parent_id() == parent_id {
                let mut mailbox_path = path.clone();
                let mailbox_role = mailbox.role();
                if mailbox_role != Role::Inbox || account.prefix.is_some() {
                    mailbox_path.push(mailbox.name().map(|n| n.to_string()).unwrap_or_default());
                } else {
                    mailbox_path.push("INBOX".to_string());
                }
                let mailbox_id = mailbox
                    .id()
                    .ok_or_else(|| {
                        jmap_client::Error::Internal(
                            "Got null mailboxId while fetching mailboxes".to_string(),
                        )
                    })?
                    .to_string();
                let has_children = result.iter().any(|child| child.parent_id() == mailbox.id());

                account.mailbox_data.insert(
                    mailbox_id.clone(),
                    Mailbox {
                        has_children,
                        is_subscribed: mailbox.is_subscribed(),
                        role: mailbox_role,
                        total_messages: mailbox.total_emails().into(),
                        total_unseen: mailbox.unread_emails().into(),
                        ..Default::default()
                    },
                );
                account
                    .mailbox_names
                    .insert(mailbox_path.join("/"), mailbox_id);

                if has_children && iter_stack.len() < 100 {
                    iter_stack.push((iter, parent_id, path));
                    parent_id = mailbox.id();
                    path = mailbox_path;
                    iter = result.iter();
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

    Ok(account)
}

impl SessionData {
    pub async fn synchronize_mailboxes(&self) -> jmap_client::Result<()> {
        // Shared mailboxes might have changed
        let mut added_accounts = Vec::new();
        if !self.client.is_session_updated() {
            self.client.refresh_session().await?;
            let session = self.client.session();

            // Remove unlinked shared accounts
            let mut added_account_ids = Vec::new();
            {
                let mut mailboxes = self.mailboxes.lock();
                let mut new_accounts = Vec::with_capacity(mailboxes.len());
                for (pos, account) in mailboxes.drain(..).enumerate() {
                    if pos == 0 || session.account(&account.account_id).is_some() {
                        new_accounts.push(account);
                    } else {
                        debug!("Removed unlinked shared account {}", account.account_id);
                    }
                }

                // Add new shared account ids
                for account_id in session.accounts() {
                    if account_id != self.client.default_account_id()
                        && !new_accounts
                            .iter()
                            .skip(1)
                            .any(|m| &m.account_id == account_id)
                    {
                        debug!("Adding shared account {}", account_id);
                        added_account_ids.push(account_id.to_string());
                    }
                }
                *mailboxes = new_accounts;
            }

            // Fetch mailboxes for each new shared account
            for account_id in added_account_ids {
                let prefix = format!(
                    "{}/{}",
                    self.core.folder_shared,
                    session.account(&account_id).unwrap().name()
                );
                match fetch_account_mailboxes(&self.client, account_id, prefix.into()).await {
                    Ok(account) => {
                        added_accounts.push(account);
                    }
                    Err(err) => {
                        debug!("Failed to fetch shared mailbox: {}", err);
                    }
                }
            }
        }

        // Fetch mailbox changes for all accounts
        let mut request = self.client.build();
        for account in self.mailboxes.lock().iter() {
            request
                .changes_mailbox(&account.state_id)
                .account_id(&account.account_id);
        }

        let mut changed_account_ids = Vec::new();
        for response in request.send().await?.unwrap_method_responses() {
            let response = match response.unwrap_changes_mailbox() {
                Ok(response) => response,
                Err(err) => {
                    debug!("Failed to fetch mailbox changes: {}", err);
                    continue;
                }
            };
            if response.total_changes() > 0 {
                if !response.created().is_empty()
                    || !response.destroyed().is_empty()
                    || (!response.updated().is_empty()
                        && response
                            .arguments()
                            .updated_properties()
                            .map_or(true, |p| p.is_empty() || p.iter().any(|p| !p.is_count())))
                {
                    changed_account_ids.push(response.unwrap_account_id());
                } else {
                    for account in self.mailboxes.lock().iter_mut() {
                        if account.account_id == response.account_id() {
                            account.state_id = response.unwrap_new_state();
                            account.mailbox_data.values_mut().for_each(|v| {
                                v.total_deleted = None;
                                v.total_unseen = None;
                                v.total_messages = None;
                                v.size = None;
                                v.uid_next = None;
                            });
                            break;
                        }
                    }
                }
            }
        }

        // Fetch mailbox data for all changed accounts
        let mut changed_accounts = Vec::with_capacity(changed_account_ids.len());
        for account_id in changed_account_ids {
            let mailbox_prefix = if account_id != self.client.default_account_id() {
                format!(
                    "{}/{}",
                    self.core.folder_shared,
                    self.client
                        .session()
                        .account(&account_id)
                        .map(|a| a.name())
                        .unwrap_or("")
                )
                .into()
            } else {
                None
            };
            match fetch_account_mailboxes(&self.client, account_id, mailbox_prefix).await {
                Ok(account_mailboxes) => {
                    changed_accounts.push(account_mailboxes);
                }
                Err(err) => {
                    debug!("Failed to fetch mailboxes: {}", err);
                }
            }
        }

        // Update mailboxes
        if !changed_accounts.is_empty() || !added_accounts.is_empty() {
            let mut mailboxes = self.mailboxes.lock();

            for changed_account in changed_accounts {
                if let Some(pos) = mailboxes
                    .iter()
                    .position(|a| a.account_id == changed_account.account_id)
                {
                    mailboxes[pos] = changed_account;
                } else {
                    mailboxes.push(changed_account);
                }
            }

            mailboxes.extend(added_accounts);
        }

        Ok(())
    }

    pub fn get_mailbox_by_name(&self, mailbox_name: &str) -> Option<MailboxData> {
        if !self.is_all_mailbox(mailbox_name) {
            for account in self.mailboxes.lock().iter() {
                if account
                    .prefix
                    .as_ref()
                    .map_or(true, |p| mailbox_name.starts_with(p))
                {
                    for (mailbox_name_, mailbox_id_) in account.mailbox_names.iter() {
                        if mailbox_name_ == mailbox_name {
                            return MailboxData {
                                account_id: account.account_id.to_string(),
                                mailbox_id: Some(mailbox_id_.to_string()),
                            }
                            .into();
                        }
                    }
                }
            }
            None
        } else {
            MailboxData {
                account_id: self.client.default_account_id().to_string(),
                mailbox_id: None,
            }
            .into()
        }
    }

    pub fn is_all_mailbox(&self, mailbox_name: &str) -> bool {
        self.core.folder_all == mailbox_name
    }
}
