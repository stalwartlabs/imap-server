use super::{
    client::SessionData,
    message::{
        increment_uid, serialize_highestmodseq, serialize_modseq, MailboxId, MODSEQ_TO_STATE,
        STATE_TO_MODSEQ,
    },
    Core,
};
use jmap_client::{
    client::Client,
    mailbox::{Property, Role},
};
use std::collections::{BTreeMap, HashMap};
use tracing::{debug, error};

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
    pub prefix: Option<String>,
    pub mailbox_state: String,
    pub mailbox_names: BTreeMap<String, String>,
    pub mailbox_data: HashMap<String, Mailbox>,
    pub modseq: Option<u32>,
}

#[derive(Debug, Default)]
pub struct MailboxSync {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub deleted: Vec<String>,
}

impl Core {
    pub async fn fetch_mailboxes(
        &self,
        client: &Client,
        folder_shared: &str,
    ) -> Option<Vec<Account>> {
        let mut mailboxes = Vec::new();

        // Fetch mailboxes for the main account
        match self
            .fetch_account_mailboxes(client, client.default_account_id().to_string(), None)
            .await
        {
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
                match self
                    .fetch_account_mailboxes(
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
        &self,
        client: &Client,
        account_id: String,
        mailbox_prefix: Option<String>,
    ) -> jmap_client::Result<Account> {
        let max_objects_in_get = client
            .session()
            .core_capabilities()
            .map(|c| c.max_objects_in_get())
            .unwrap_or(500);
        let mut position = 0;
        let mut result = Vec::with_capacity(10);
        let mut mailbox_state = String::new();

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
            mailbox_state = get_response.take_state();
            let mailboxes_part = get_response.take_list();
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
            prefix: mailbox_prefix,
            mailbox_names: BTreeMap::new(),
            mailbox_data: HashMap::with_capacity(result.len()),
            mailbox_state,
            modseq: None,
        };

        // Build list item tree
        loop {
            while let Some(mailbox) = iter.next() {
                if mailbox.parent_id() == parent_id {
                    let mut mailbox_path = path.clone();
                    let mailbox_role = mailbox.role();
                    if mailbox_role != Role::Inbox || account.prefix.is_some() {
                        mailbox_path
                            .push(mailbox.name().map(|n| n.to_string()).unwrap_or_default());
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
}

impl SessionData {
    pub async fn synchronize_mailboxes(
        &self,
        return_changes: bool,
        force_session_refresh: bool,
    ) -> jmap_client::Result<Option<MailboxSync>> {
        let mut changes = if return_changes {
            MailboxSync::default().into()
        } else {
            None
        };

        // Shared mailboxes might have changed
        let mut added_accounts = Vec::new();
        if force_session_refresh || !self.client.is_session_updated() {
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

                        // Add unshared mailboxes to deleted list
                        if let Some(changes) = &mut changes {
                            for (mailbox_name, _) in account.mailbox_names {
                                changes.deleted.push(mailbox_name);
                            }
                        }
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
                match self
                    .core
                    .fetch_account_mailboxes(&self.client, account_id, prefix.into())
                    .await
                {
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
                .changes_mailbox(&account.mailbox_state)
                .account_id(&account.account_id);
        }

        let mut changed_account_ids = Vec::new();
        for response in request.send().await?.unwrap_method_responses() {
            let mut response = match response.unwrap_changes_mailbox() {
                Ok(response) => response,
                Err(err) => {
                    debug!("Failed to fetch mailbox changes: {}", err);
                    continue;
                }
            };
            if response.total_changes() > 0 {
                let reset_stats = if !response.created().is_empty()
                    || !response.destroyed().is_empty()
                    || (!response.updated().is_empty()
                        && response
                            .arguments()
                            .updated_properties()
                            .map_or(true, |p| p.is_empty() || p.iter().any(|p| !p.is_count())))
                    || changes.is_some()
                {
                    changed_account_ids.push(response.take_account_id());
                    false
                } else {
                    true
                };

                for account in self.mailboxes.lock().iter_mut() {
                    if account.account_id == response.account_id() {
                        account.mailbox_state = response.take_new_state();
                        if reset_stats {
                            account.mailbox_data.values_mut().for_each(|v| {
                                v.total_deleted = None;
                                v.total_unseen = None;
                                v.total_messages = None;
                                v.size = None;
                                v.uid_next = None;
                                account.modseq = None;
                            });
                        }
                        break;
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
            match self
                .core
                .fetch_account_mailboxes(&self.client, account_id, mailbox_prefix)
                .await
            {
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
                    // Add changes and deletions
                    if let Some(changes) = &mut changes {
                        let old_account = &mailboxes[pos];
                        let new_account = &changed_account;

                        // Add new mailboxes
                        for (mailbox_name, mailbox_id) in new_account.mailbox_names.iter() {
                            if let Some(old_mailbox) = old_account.mailbox_data.get(mailbox_id) {
                                if let Some(mailbox) = new_account.mailbox_data.get(mailbox_id) {
                                    if mailbox.total_messages.unwrap_or(0)
                                        != old_mailbox.total_messages.unwrap_or(0)
                                        || mailbox.total_unseen.unwrap_or(0)
                                            != old_mailbox.total_unseen.unwrap_or(0)
                                    {
                                        changes.changed.push(mailbox_name.to_string());
                                    }
                                }
                            } else {
                                changes.added.push(mailbox_name.to_string());
                            }
                        }

                        // Add deleted mailboxes
                        for (mailbox_name, mailbox_id) in &old_account.mailbox_names {
                            if !new_account.mailbox_data.contains_key(mailbox_id) {
                                changes.deleted.push(mailbox_name.to_string());
                            }
                        }
                    }

                    mailboxes[pos] = changed_account;
                } else {
                    // Add newly shared accounts
                    if let Some(changes) = &mut changes {
                        changes
                            .added
                            .extend(changed_account.mailbox_names.keys().cloned());
                    }

                    mailboxes.push(changed_account);
                }
            }

            if !added_accounts.is_empty() {
                // Add newly shared accounts
                if let Some(changes) = &mut changes {
                    for added_account in &added_accounts {
                        changes
                            .added
                            .extend(added_account.mailbox_names.keys().cloned());
                    }
                }
                mailboxes.extend(added_accounts);
            }
        }

        Ok(changes)
    }

    pub fn get_mailbox_by_name(&self, mailbox_name: &str) -> Option<MailboxId> {
        if !self.is_all_mailbox(mailbox_name) {
            for account in self.mailboxes.lock().iter() {
                if account
                    .prefix
                    .as_ref()
                    .map_or(true, |p| mailbox_name.starts_with(p))
                {
                    for (mailbox_name_, mailbox_id_) in account.mailbox_names.iter() {
                        if mailbox_name_ == mailbox_name {
                            return MailboxId {
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
            MailboxId {
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

impl Core {
    pub async fn state_to_modseq(&self, account_id: &str, state: String) -> Result<u32, ()> {
        let highestmodseq_key = serialize_highestmodseq(account_id.as_bytes());
        let modseq_key = serialize_modseq(account_id.as_bytes(), state.as_bytes(), STATE_TO_MODSEQ);
        let db = self.db.clone();
        self.spawn_worker(move || {
            let modseq = if let Some(modseq) = db.get(&modseq_key).map_err(|err| {
                error!("Failed to get key: {}", err);
            })? {
                modseq
            } else {
                // Obtain highestmodseq.
                let highestmodseq = db
                    .update_and_fetch(&highestmodseq_key, increment_uid)
                    .map_err(|err| {
                        error!("Failed to increment HIGHESTMODSEQ: {}", err);
                    })?
                    .ok_or_else(|| {
                        error!("Failed to generate HIGHESTMODSEQ.");
                    })?;

                // Insert state-to-modseq key
                db.insert(modseq_key, &highestmodseq).map_err(|err| {
                    error!("Failed to insert key: {}", err);
                })?;
                // Insert modseq-to-state key
                db.insert(
                    serialize_modseq(
                        &highestmodseq_key[..highestmodseq_key.len() - 2],
                        &highestmodseq[..],
                        MODSEQ_TO_STATE,
                    ),
                    state.as_bytes(),
                )
                .map_err(|err| {
                    error!("Failed to insert key: {}", err);
                })?;

                highestmodseq
            };

            Ok(u32::from_be_bytes((&modseq[..]).try_into().map_err(
                |err| {
                    error!("Failed to decode UID validity: {}", err);
                },
            )?))
        })
        .await
    }

    pub async fn modseq_to_state(
        &self,
        account_id: &str,
        modseq: u32,
    ) -> Result<Option<String>, ()> {
        let modseq_key = serialize_modseq(
            account_id.as_bytes(),
            &modseq.to_be_bytes(),
            MODSEQ_TO_STATE,
        );
        let db = self.db.clone();
        self.spawn_worker(move || {
            Ok(
                if let Some(state) = db.get(&modseq_key).map_err(|err| {
                    error!("Failed to get key: {}", err);
                })? {
                    String::from_utf8(state.to_vec())
                        .map_err(|err| {
                            error!("Failed to convert state to string: {}", err);
                        })?
                        .into()
                } else {
                    None
                },
            )
        })
        .await
    }
}
