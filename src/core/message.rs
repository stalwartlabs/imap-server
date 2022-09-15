/*
 * Copyright (c) 2020-2022, Stalwart Labs Ltd.
 *
 * This file is part of the Stalwart IMAP Server.
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of
 * the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 * in the LICENSE file at the top-level directory of this distribution.
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/>.
 *
 * You can be released from the requirements of the AGPLv3 license by
 * purchasing a commercial license. Please contact licensing@stalw.art
 * for more details.
*/

use std::{sync::Arc, time::SystemTime};

use ahash::{AHashMap, AHashSet};
use jmap_client::email::query::Filter;
use tokio::sync::oneshot;
use tracing::{debug, error};

use crate::protocol::Sequence;

use super::{
    client::{SelectedMailbox, SessionData},
    mailbox::Account,
    Core, IntoStatusResponse, StatusResponse,
};

#[derive(Debug, PartialEq, Eq)]
pub struct MailboxId {
    pub account_id: String,
    pub mailbox_id: Option<String>,
}

#[derive(Debug)]
pub struct MailboxData {
    pub uid_next: u32,
    pub uid_validity: u32,
    pub jmap_ids: Vec<String>,
    pub imap_uids: Vec<u32>,
    pub total_messages: usize,
    pub last_state: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ImapId {
    pub uid: u32,
    pub seqnum: u32,
}

pub const JMAP_TO_UID: u8 = 0;
pub const UID_TO_JMAP: u8 = 1;
pub const UID_NEXT: u8 = 2;
pub const UID_VALIDITY: u8 = 3;
pub const MODSEQ_TO_STATE: u8 = 4;
pub const STATE_TO_MODSEQ: u8 = 5;
pub const HIGHEST_MODSEQ: u8 = 6;
pub const JMAP_DELETED_IDS: u8 = 7;

impl SessionData {
    pub async fn synchronize_messages(
        &self,
        mailbox: Arc<MailboxId>,
    ) -> Result<MailboxData, StatusResponse> {
        let mut valid_ids = Vec::new();
        let mut position = 0;

        // Fetch all ids in the mailbox.
        for _ in 0..100 {
            let mut request = self.client.build().account_id(&mailbox.account_id);
            let query_request = request
                .query_email()
                .calculate_total(true)
                .position(position);
            if let Some(mailbox_id) = &mailbox.mailbox_id {
                query_request.filter(Filter::in_mailbox(mailbox_id));
            }

            let mut response = request
                .send_query_email()
                .await
                .map_err(|err| err.into_status_response())?;
            let total_messages = response.total().unwrap_or(0);
            let emails = response.take_ids();

            let emails_len = emails.len();
            if emails_len > 0 {
                valid_ids.extend(emails);
                if valid_ids.len() < total_messages {
                    position += emails_len as i32;
                    continue;
                }
            }
            break;
        }

        // Update mailbox
        self.core
            .update_uids(mailbox, valid_ids)
            .await
            .map_err(|_| StatusResponse::database_failure())
    }

    pub async fn get_jmap_state(&self, account_id: &str) -> Result<String, StatusResponse> {
        let mut request = self.client.build();
        request
            .get_email()
            .account_id(account_id)
            .ids(Vec::<&str>::new());
        request
            .send_get_email()
            .await
            .map_err(|err| err.into_status_response())
            .map(|mut r| r.take_state())
    }

    pub async fn synchronize_state(&self, account_id: &str) -> Result<u32, StatusResponse> {
        // Update modseq
        self.core
            .state_to_modseq(account_id, self.get_jmap_state(account_id).await?)
            .await
            .map_err(|_| StatusResponse::database_failure())
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum MappingOptions {
    None,
    AddIfMissing,
    IncludeDeleted,
    OnlyIncludeDeleted,
}

impl Core {
    pub async fn update_uids(
        &self,
        mailbox: Arc<MailboxId>,
        mut update_jmap_ids: Vec<String>,
    ) -> Result<MailboxData, ()> {
        let db = self.db.clone();
        self.spawn_worker(move || {
            // Obtain/generate UIDVALIDITY
            let uid_validity = db.uid_validity(&mailbox)?;

            // Remove from cache messages no longer present in the mailbox.
            let mut jmap_ids_map = update_jmap_ids
                .iter()
                .enumerate()
                .map(|(pos, id)| (id.as_bytes(), pos))
                .collect::<AHashMap<_, _>>();
            let mut imap_uids = Vec::with_capacity(update_jmap_ids.len());
            let mut jmap_ids = Vec::with_capacity(update_jmap_ids.len());
            let mut found_ids = vec![0u8; update_jmap_ids.len()];

            let prefix = serialize_key_prefix(&mailbox, UID_TO_JMAP);
            let mut batch = sled::Batch::default();
            let mut has_deletions = false;

            for kv_result in db.scan_prefix(&prefix) {
                let (key, value) = kv_result.map_err(|err| {
                    error!("Failed to scan db: {}", err);
                })?;
                if key.len() > prefix.len() {
                    let imap_uid = &key[prefix.len()..];
                    let jmap_id = &value[..];

                    if let Some(pos) = jmap_ids_map.remove(jmap_id) {
                        imap_uids.push(u32::from_be_bytes(imap_uid.try_into().map_err(|_| {
                            error!("Failed to convert bytes to u32.");
                        })?));
                        jmap_ids.push(String::from_utf8(value.to_vec()).map_err(|_| {
                            error!("Failed to convert bytes to string.");
                        })?);
                        found_ids[pos] = 1;
                    } else {
                        // Add UID to deleted messages
                        let mut buf = Vec::with_capacity(
                            std::mem::size_of::<u32>() + std::mem::size_of::<u64>(),
                        );
                        buf.extend_from_slice(imap_uid);
                        buf.extend_from_slice(
                            &SystemTime::now()
                                .duration_since(SystemTime::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0)
                                .to_be_bytes(),
                        );
                        batch.insert(serialize_key(&mailbox, JMAP_DELETED_IDS, jmap_id), buf);

                        // Delete mappings from cache
                        batch.remove(key);
                        batch.remove(sled::IVec::from(serialize_key(
                            &mailbox,
                            JMAP_TO_UID,
                            jmap_id,
                        )));

                        has_deletions = true;
                    }
                }
            }

            if has_deletions {
                db.apply_batch(batch).map_err(|err| {
                    error!("Failed to delete batch: {}", err);
                })?;
            }

            // Add to the db any new ids.
            if !jmap_ids_map.is_empty() {
                let uid_next_key = serialize_uid_next_key(&mailbox);

                for (pos, found) in found_ids.into_iter().enumerate() {
                    if found == 0 {
                        let jmap_id = std::mem::take(update_jmap_ids.get_mut(pos).unwrap());
                        let imap_uid =
                            db.insert_jmap_id(&mailbox, jmap_id.as_bytes(), &uid_next_key)?;
                        jmap_ids.push(jmap_id);
                        imap_uids.push(u32::from_be_bytes((&imap_uid[..]).try_into().map_err(
                            |_| {
                                error!("Failed to convert bytes to u32.");
                            },
                        )?));
                    }
                }
            }

            Ok(MailboxData {
                uid_validity,
                uid_next: db.uid_next(&mailbox)?,
                total_messages: imap_uids.len(),
                jmap_ids,
                imap_uids,
                last_state: String::new(),
            })
        })
        .await
    }

    pub async fn jmap_to_imap(
        &self,
        mailbox: Arc<MailboxId>,
        update_jmap_ids: Vec<String>,
        options: MappingOptions,
    ) -> Result<(Vec<String>, Vec<u32>), ()> {
        let db = self.db.clone();
        self.spawn_worker(move || {
            let mut jmap_ids = Vec::with_capacity(update_jmap_ids.len());
            let mut imap_uids = Vec::with_capacity(update_jmap_ids.len());
            let mut uid_next_key = None;

            for jmap_id in update_jmap_ids {
                let jmap_id_bytes = jmap_id.as_bytes();

                if options != MappingOptions::OnlyIncludeDeleted {
                    if let Some(uid) = db
                        .get(serialize_key(&mailbox, JMAP_TO_UID, jmap_id_bytes))
                        .map_err(|err| {
                            error!("Failed to get key: {}", err);
                        })?
                    {
                        jmap_ids.push(jmap_id);
                        imap_uids.push(u32::from_be_bytes((&uid[..]).try_into().map_err(
                            |_| {
                                error!("Failed to convert bytes to u32.");
                            },
                        )?));
                        continue;
                    } else if options == MappingOptions::AddIfMissing {
                        let uid = db.insert_jmap_id(
                            &mailbox,
                            jmap_id_bytes,
                            uid_next_key.get_or_insert_with(|| serialize_uid_next_key(&mailbox)),
                        )?;
                        jmap_ids.push(jmap_id);
                        imap_uids.push(u32::from_be_bytes((&uid[..]).try_into().map_err(
                            |_| {
                                error!("Failed to convert bytes to u32.");
                            },
                        )?));
                        continue;
                    } else if options != MappingOptions::IncludeDeleted {
                        continue;
                    }
                }

                if let Some(uid) = db
                    .get(serialize_key(&mailbox, JMAP_DELETED_IDS, jmap_id_bytes))
                    .map_err(|err| {
                        error!("Failed to get key: {}", err);
                    })?
                {
                    imap_uids.push(u32::from_be_bytes(
                        (&uid[..std::mem::size_of::<u32>()])
                            .try_into()
                            .map_err(|_| {
                                error!("Failed to convert bytes to u32.");
                            })?,
                    ));
                }
            }

            Ok((jmap_ids, imap_uids))
        })
        .await
    }

    pub async fn imap_to_jmap(
        &self,
        mailbox: Arc<MailboxId>,
        imap_ids: Vec<u32>,
    ) -> Result<(Vec<String>, Vec<u32>), ()> {
        let db = self.db.clone();
        self.spawn_worker(move || {
            let mut jmap_ids = Vec::with_capacity(imap_ids.len());
            let mut imap_uids = Vec::with_capacity(imap_ids.len());
            for uid in imap_ids {
                if let Some(jmap_id) = db
                    .get(serialize_key(&mailbox, UID_TO_JMAP, &uid.to_be_bytes()[..]))
                    .map_err(|err| {
                        error!("Failed to get key: {}", err);
                    })?
                {
                    imap_uids.push(uid);
                    jmap_ids.push(String::from_utf8(jmap_id.to_vec()).map_err(|_| {
                        error!("Failed to convert bytes to string.");
                    })?);
                }
            }
            Ok((jmap_ids, imap_uids))
        })
        .await
    }

    pub async fn delete_ids(
        &self,
        mailbox: Arc<MailboxId>,
        jmap_ids: Vec<String>,
    ) -> Result<(), ()> {
        let db = self.db.clone();
        self.spawn_worker(move || {
            let mut batch = sled::Batch::default();
            let mut has_deletions = false;

            for jmap_id in jmap_ids {
                let jmap_id = jmap_id.as_bytes();
                let key = serialize_key(&mailbox, JMAP_TO_UID, jmap_id);

                if let Some(imap_uid) = db.get(&key).map_err(|err| {
                    error!("Failed to get key: {}", err);
                })? {
                    // Add UID to deleted messages
                    let mut buf =
                        Vec::with_capacity(std::mem::size_of::<u32>() + std::mem::size_of::<u64>());
                    buf.extend_from_slice(&imap_uid[..]);
                    buf.extend_from_slice(
                        &SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0)
                            .to_be_bytes(),
                    );
                    batch.insert(serialize_key(&mailbox, JMAP_DELETED_IDS, jmap_id), buf);

                    // Delete mappings from cache
                    batch.remove(key);
                    batch.remove(sled::IVec::from(serialize_key(
                        &mailbox,
                        UID_TO_JMAP,
                        &imap_uid[..],
                    )));

                    has_deletions = true;
                }
            }

            if has_deletions {
                db.apply_batch(batch).map_err(|err| {
                    error!("Failed to delete batch: {}", err);
                })?;
            }

            Ok(())
        })
        .await
    }

    pub async fn delete_account(&self, account_id: String) -> Result<(), ()> {
        let db = self.db.clone();
        self.spawn_worker(move || {
            let mut batch = sled::Batch::default();

            for kv_result in db.scan_prefix(&serialize_key_account_prefix(&account_id)) {
                let (key, _) = kv_result.map_err(|err| {
                    error!("Failed to scan db: {}", err);
                })?;
                batch.remove(key);
            }

            db.apply_batch(batch).map_err(|err| {
                error!("Failed to delete batch: {}", err);
            })?;

            Ok(())
        })
        .await
    }

    pub async fn delete_mailbox(&self, account_id: &str, mailbox_id: &str) -> Result<(), ()> {
        let mut prefix = serialize_key_account_prefix(account_id);
        prefix.extend_from_slice(mailbox_id.as_bytes());

        let db = self.db.clone();
        self.spawn_worker(move || {
            let mut batch = sled::Batch::default();

            for kv_result in db.scan_prefix(&prefix) {
                let (key, _) = kv_result.map_err(|err| {
                    error!("Failed to scan db: {}", err);
                })?;
                if key.len() > prefix.len() && key[prefix.len()] <= UID_VALIDITY {
                    batch.remove(key);
                }
            }

            db.apply_batch(batch).map_err(|err| {
                error!("Failed to delete batch: {}", err);
            })?;

            Ok(())
        })
        .await
    }

    pub async fn uids(&self, mailbox: Arc<MailboxId>) -> Result<(u32, u32), ()> {
        let db = self.db.clone();
        self.spawn_worker(move || Ok((db.uid_validity(&mailbox)?, db.uid_next(&mailbox)?)))
            .await
    }

    pub async fn purge_deleted_mailboxes(&self, account: &Account) -> Result<(), ()> {
        if account.mailbox_data.is_empty() {
            debug!(
                "No mailboxes found for account '{}', skipping purge.",
                account.account_id
            );
            return Ok(());
        }
        let account_prefix = serialize_key_account_prefix(&account.account_id);
        let mailbox_keys = account
            .mailbox_data
            .keys()
            .map(|id| id.as_bytes().to_vec())
            .collect::<AHashSet<_>>();

        let db = self.db.clone();
        self.spawn_worker(move || {
            let mut has_deletions = false;
            let mut batch = sled::Batch::default();

            for kv_result in db.scan_prefix(&account_prefix) {
                let (key, _) = kv_result.map_err(|err| {
                    error!("Failed to scan db: {}", err);
                })?;
                let key_part = &key[account_prefix.len()..];
                if let Some(pos) = key_part.iter().position(|&ch| ch <= UID_VALIDITY) {
                    if pos > 0 && !mailbox_keys.contains(&key_part[..pos]) {
                        batch.remove(key);
                        has_deletions = true;
                    }
                }
            }

            if has_deletions {
                db.apply_batch(batch).map_err(|err| {
                    error!("Failed to delete batch: {}", err);
                })?;
            }

            Ok(())
        })
        .await
    }

    pub async fn purge_deleted_ids(&self, ttl: u64) -> Result<usize, ()> {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .map_err(|err| {
                error!("Failed to obtain current time: {}", err);
            })?;

        let db = self.db.clone();
        self.spawn_worker(move || {
            let mut num_deletions = 0;
            let mut batch = sled::Batch::default();

            for kv_result in db.scan_prefix(&[]) {
                let (key, value) = kv_result.map_err(|err| {
                    error!("Failed to scan db: {}", err);
                })?;
                if value.len() == std::mem::size_of::<u32>() + std::mem::size_of::<u64>() {
                    let insert_time = u64::from_be_bytes(
                        (&value[std::mem::size_of::<u32>()..])
                            .try_into()
                            .map_err(|_| {
                                error!("Failed to convert bytes to u32.");
                            })?,
                    );
                    if insert_time < now && (now - insert_time) >= ttl {
                        batch.remove(key);
                        num_deletions += 1;
                    }
                }
            }

            if num_deletions > 0 {
                db.apply_batch(batch).map_err(|err| {
                    error!("Failed to delete batch: {}", err);
                })?;
            }

            Ok(num_deletions)
        })
        .await
    }

    pub async fn spawn_worker<U, V>(&self, f: U) -> Result<V, ()>
    where
        U: FnOnce() -> Result<V, ()> + Send + 'static,
        V: Sync + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();

        self.worker_pool.spawn(move || {
            tx.send(f()).ok();
        });

        rx.await.map_err(|e| {
            error!("Await error: {}", e);
        })?
    }
}

impl SelectedMailbox {
    pub async fn sequence_to_jmap(
        &self,
        sequence: &Sequence,
        is_uid: bool,
    ) -> crate::core::Result<AHashMap<String, ImapId>> {
        if !sequence.is_saved_search() {
            let mut ids = AHashMap::new();
            let state = self.state.lock();
            if state.imap_uids.is_empty() {
                return Ok(ids);
            }

            let max_uid = state.imap_uids.last().copied().unwrap_or(0) as u32;
            let max_seqnum = state.imap_uids.len() as u32;

            for (pos, &uid) in state.imap_uids.iter().enumerate() {
                if uid != 0 {
                    let matched = if is_uid {
                        sequence.contains(uid, max_uid)
                    } else {
                        sequence.contains((pos + 1) as u32, max_seqnum)
                    };
                    if matched {
                        ids.insert(
                            state.jmap_ids[pos].clone(),
                            ImapId::new(uid, (pos + 1) as u32),
                        );
                    }
                }
            }

            Ok(ids)
        } else {
            let saved_ids = self
                .get_saved_search()
                .await
                .ok_or_else(|| StatusResponse::no("No saved search found."))?;
            let mut ids = AHashMap::with_capacity(saved_ids.len());
            let state = self.state.lock();

            for imap_id in saved_ids.iter() {
                if state.imap_uids.contains(&imap_id.uid) {
                    ids.insert(
                        state.jmap_ids[imap_id.seqnum.saturating_sub(1) as usize].clone(),
                        *imap_id,
                    );
                }
            }

            Ok(ids)
        }
    }

    pub fn jmap_to_imap(&self, jmap_ids: &[String]) -> Vec<ImapId> {
        let mut imap_ids = Vec::with_capacity(jmap_ids.len());
        let state = self.state.lock();

        for jmap_id in jmap_ids {
            if let Some(seqnum) = state.jmap_ids.iter().position(|id| id == jmap_id) {
                imap_ids.push(ImapId::new(state.imap_uids[seqnum], (seqnum + 1) as u32));
            }
        }

        imap_ids
    }

    pub fn imap_to_jmap(&self, imap_ids: &[ImapId]) -> Vec<String> {
        let mut jmap_ids = Vec::with_capacity(imap_ids.len());
        let state = self.state.lock();

        for imap_id in imap_ids {
            if let Some(pos) = state.imap_uids.iter().position(|uid| *uid == imap_id.uid) {
                jmap_ids.push(state.jmap_ids[pos].clone());
            }
        }

        jmap_ids
    }

    pub fn is_in_sync(&self, jmap_ids: &[String]) -> bool {
        let state = self.state.lock();

        for jmap_id in jmap_ids {
            if !state.jmap_ids.contains(jmap_id) {
                return false;
            }
        }
        true
    }

    pub fn synchronize_uids(
        &self,
        jmap_ids: Vec<String>,
        imap_uids: Vec<u32>,
        remove_missing: bool,
    ) -> (Option<usize>, Option<Vec<ImapId>>) {
        let mut state = self.state.lock();
        let mut has_inserts = false;

        let deletions = if remove_missing {
            let mut deletions = Vec::new();

            let mut new_jmap_ids = Vec::with_capacity(state.jmap_ids.len());
            let mut new_imap_uids = Vec::with_capacity(state.imap_uids.len());

            for (pos, (jmap_id, imap_uid)) in std::mem::take(&mut state.jmap_ids)
                .into_iter()
                .zip(std::mem::take(&mut state.imap_uids))
                .enumerate()
            {
                if imap_uids.contains(&imap_uid) {
                    new_jmap_ids.push(jmap_id);
                    new_imap_uids.push(imap_uid);
                } else {
                    deletions.push(ImapId::new(imap_uid, (pos + 1) as u32));
                }
            }

            state.jmap_ids = new_jmap_ids;
            state.imap_uids = new_imap_uids;

            if !deletions.is_empty() {
                state.total_messages = state.total_messages.saturating_sub(deletions.len());
                deletions.into()
            } else {
                None
            }
        } else {
            None
        };

        for (pos, jmap_id) in jmap_ids.into_iter().enumerate() {
            let uid = imap_uids[pos];
            if !state.imap_uids.contains(&uid) {
                state.imap_uids.push(uid);
                state.jmap_ids.push(jmap_id);
                state.total_messages += 1;
                has_inserts = true;
            }
        }
        (
            if has_inserts || deletions.is_some() {
                state.total_messages.into()
            } else {
                None
            },
            deletions,
        )
    }
}

impl ImapId {
    pub fn new(uid: u32, seqnum: u32) -> Self {
        ImapId { uid, seqnum }
    }
}

trait ImapUtils {
    fn insert_jmap_id(
        &self,
        mailbox: &MailboxId,
        jmap_id: &[u8],
        uid_next_key: &[u8],
    ) -> Result<sled::IVec, ()>;
    fn uid_next(&self, mailbox: &MailboxId) -> Result<u32, ()>;
    fn uid_validity(&self, mailbox: &MailboxId) -> Result<u32, ()>;
}

impl ImapUtils for sled::Db {
    fn insert_jmap_id(
        &self,
        mailbox: &MailboxId,
        jmap_id: &[u8],
        uid_next_key: &[u8],
    ) -> Result<sled::IVec, ()> {
        // Obtain next UID.
        let uid = self
            .update_and_fetch(&uid_next_key, increment_uid)
            .map_err(|err| {
                error!("Failed to increment UID: {}", err);
            })?
            .ok_or_else(|| {
                error!("Failed to generate UID.");
            })?;

        // Write keys
        for result in [
            self.insert(serialize_key(mailbox, JMAP_TO_UID, jmap_id), &uid),
            self.insert(serialize_key(mailbox, UID_TO_JMAP, &uid), jmap_id),
        ] {
            result.map_err(|err| {
                error!("Failed to insert key: {}", err);
            })?;
        }
        Ok(uid)
    }

    fn uid_next(&self, mailbox: &MailboxId) -> Result<u32, ()> {
        Ok(
            if let Some(uid_bytes) = self.get(serialize_uid_next_key(mailbox)).map_err(|err| {
                error!("Failed to read key: {}", err);
            })? {
                u32::from_be_bytes((&uid_bytes[..]).try_into().map_err(|err| {
                    error!("Failed to decode UID next: {}", err);
                })?) + 1
            } else {
                1
            },
        )
    }

    fn uid_validity(&self, mailbox: &MailboxId) -> Result<u32, ()> {
        // Obtain/generate UIDVALIDITY
        let uid_validity_key = serialize_uid_validity_key(mailbox);
        Ok(
            if let Some(uid_bytes) = self.get(&uid_validity_key).map_err(|err| {
                error!("Failed to read key: {}", err);
            })? {
                u32::from_be_bytes((&uid_bytes[..]).try_into().map_err(|err| {
                    error!("Failed to decode UID validity: {}", err);
                })?)
            } else {
                // Number of hours since January 1st, 2000
                let uid_validity = (SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
                    .saturating_sub(946684800)
                    / 3600) as u32;
                self.insert(uid_validity_key, &uid_validity.to_be_bytes()[..])
                    .map_err(|err| {
                        error!("Failed to insert key: {}", err);
                    })?;
                uid_validity
            },
        )
    }
}

fn serialize_key(mailbox: &MailboxId, separator: u8, value: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(
        mailbox.account_id.len()
            + mailbox.mailbox_id.as_ref().map_or(0, |m| m.len())
            + value.len()
            + 2,
    );
    buf.extend_from_slice(mailbox.account_id.as_bytes());
    buf.push(0);
    if let Some(mailbox_id) = mailbox.mailbox_id.as_ref() {
        buf.extend_from_slice(mailbox_id.as_bytes());
    }
    buf.push(separator);
    buf.extend_from_slice(value);
    buf
}

fn serialize_key_prefix(mailbox: &MailboxId, separator: u8) -> Vec<u8> {
    let mut buf = Vec::with_capacity(
        mailbox.account_id.len() + mailbox.mailbox_id.as_ref().map_or(0, |m| m.len()) + 2,
    );
    buf.extend_from_slice(mailbox.account_id.as_bytes());
    buf.push(0);
    if let Some(mailbox_id) = mailbox.mailbox_id.as_ref() {
        buf.extend_from_slice(mailbox_id.as_bytes());
    }
    buf.push(separator);
    buf
}

fn serialize_key_account_prefix(account_id: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(account_id.len() + 1);
    buf.extend_from_slice(account_id.as_bytes());
    buf.push(0);
    buf
}

fn serialize_uid_next_key(mailbox: &MailboxId) -> Vec<u8> {
    let mut buf = Vec::with_capacity(
        mailbox.account_id.len() + mailbox.mailbox_id.as_ref().map_or(0, |m| m.len()) + 2,
    );
    buf.extend_from_slice(mailbox.account_id.as_bytes());
    buf.push(0);
    if let Some(mailbox_id) = mailbox.mailbox_id.as_ref() {
        buf.extend_from_slice(mailbox_id.as_bytes());
    }
    buf.push(UID_NEXT);
    buf
}

fn serialize_uid_validity_key(mailbox: &MailboxId) -> Vec<u8> {
    let mut buf = Vec::with_capacity(
        mailbox.account_id.len() + mailbox.mailbox_id.as_ref().map_or(0, |m| m.len()) + 2,
    );
    buf.extend_from_slice(mailbox.account_id.as_bytes());
    buf.push(0);
    if let Some(mailbox_id) = mailbox.mailbox_id.as_ref() {
        buf.extend_from_slice(mailbox_id.as_bytes());
    }
    buf.push(UID_VALIDITY);
    buf
}

pub fn serialize_modseq(account_id: &[u8], value: &[u8], separator: u8) -> Vec<u8> {
    let mut buf = Vec::with_capacity(account_id.len() + value.len() + 2);
    buf.extend_from_slice(account_id);
    buf.push(0);
    buf.extend_from_slice(value);
    buf.push(separator);
    buf
}

pub fn serialize_highestmodseq(account_id: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(account_id.len() + 2);
    buf.extend_from_slice(account_id);
    buf.push(0);
    buf.push(HIGHEST_MODSEQ);
    buf
}

pub fn increment_uid(old: Option<&[u8]>) -> Option<Vec<u8>> {
    match old {
        Some(bytes) => u32::from_be_bytes(bytes.try_into().ok()?) + 1,
        None => 1,
    }
    .to_be_bytes()
    .to_vec()
    .into()
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc, time::Duration};

    use ahash::AHashMap;

    use crate::{
        core::{
            config::build_core,
            mailbox::{Account, Mailbox},
            message::MappingOptions,
        },
        tests::init_settings,
    };

    use super::MailboxId;

    #[tokio::test]
    async fn synchronize_messages() {
        let (settings, temp_dir) = init_settings(true);
        let core = build_core(&settings);

        // Initial test data
        let mailbox = Arc::new(MailboxId {
            account_id: "john".to_string(),
            mailbox_id: "inbox_id".to_string().into(),
        });
        let mailbox_abc = Arc::new(MailboxId {
            account_id: "abc".to_string(),
            mailbox_id: "inbox_id".to_string().into(),
        });
        let mailbox_xyz = Arc::new(MailboxId {
            account_id: "xyz".to_string(),
            mailbox_id: "inbox_id".to_string().into(),
        });
        let jmap_ids = vec![
            "a00".to_string(),
            "b01".to_string(),
            "c02".to_string(),
            "d03".to_string(),
            "e04".to_string(),
            "f05".to_string(),
            "g06".to_string(),
            "h07".to_string(),
            "i08".to_string(),
            "j09".to_string(),
        ];
        let uids = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

        // Insert test data
        let update_result = core
            .update_uids(mailbox.clone(), jmap_ids.clone())
            .await
            .unwrap();
        assert_eq!(update_result.uid_next, 11);

        let update_result = core
            .update_uids(mailbox_abc.clone(), jmap_ids.clone())
            .await
            .unwrap();
        assert_eq!(update_result.uid_next, 11);

        let update_result = core
            .update_uids(mailbox_xyz.clone(), jmap_ids.clone())
            .await
            .unwrap();
        assert_eq!(update_result.uid_next, 11);

        // Check generated UIDs
        assert_eq!(
            core.jmap_to_imap(mailbox.clone(), jmap_ids.clone(), MappingOptions::None)
                .await
                .unwrap()
                .1,
            uids
        );
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), uids.clone())
                .await
                .unwrap()
                .0,
            jmap_ids
        );

        // Remove account
        core.delete_account("abc".to_string()).await.unwrap();
        let (uid_validity, uid_next) = core.uids(mailbox_abc.clone()).await.unwrap();
        assert_ne!(uid_validity, 0);
        assert_eq!(uid_next, 1);
        assert_eq!(
            core.imap_to_jmap(mailbox_abc.clone(), vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10],)
                .await
                .unwrap()
                .0,
            Vec::<String>::new()
        );

        // Remove and add messages
        let jmap_ids = [
            "a00", "b01", "c02", "h07", "i08", "j09", "h10", "i11", "j12", "k13",
        ]
        .into_iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>();
        let uids = vec![1, 2, 3, 8, 9, 10, 11, 12, 13, 14];

        let update_result = core
            .update_uids(mailbox.clone(), jmap_ids.clone())
            .await
            .unwrap();
        assert_eq!(update_result.uid_next, 15);

        // Check IDs
        assert_eq!(
            core.jmap_to_imap(mailbox.clone(), jmap_ids.clone(), MappingOptions::None)
                .await
                .unwrap()
                .1,
            uids
        );

        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), uids.clone())
                .await
                .unwrap()
                .0,
            jmap_ids
        );

        // Non existant UIDs
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10])
                .await
                .unwrap()
                .0,
            vec![
                "a00".to_string(),
                "b01".to_string(),
                "c02".to_string(),
                "h07".to_string(),
                "i08".to_string(),
                "j09".to_string()
            ]
        );

        // Remove all ids and add some new ids later
        let update_result = core.update_uids(mailbox.clone(), vec![]).await.unwrap();
        assert_eq!(update_result.uid_next, 15);
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), vec![1, 8, 15])
                .await
                .unwrap()
                .0,
            Vec::<String>::new()
        );
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), vec![1, 5, 10])
                .await
                .unwrap()
                .0,
            Vec::<String>::new()
        );
        let update_result = core
            .update_uids(mailbox.clone(), vec!["x01".to_string(), "y02".to_string()])
            .await
            .unwrap();
        assert_eq!(update_result.uid_next, 17);
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), vec![15, 16])
                .await
                .unwrap()
                .0,
            vec!["x01".to_string(), "y02".to_string(),]
        );

        // Test deleted ids purge
        assert_eq!(core.purge_deleted_ids(1).await.unwrap(), 0);
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert_eq!(core.purge_deleted_ids(1).await.unwrap(), 14);

        // Test mailbox purge
        let mailbox_2 = Arc::new(MailboxId {
            account_id: "john".to_string(),
            mailbox_id: "folder_id".to_string().into(),
        });
        let jmap_ids = vec![
            "a00".to_string(),
            "b01".to_string(),
            "c02".to_string(),
            "d03".to_string(),
            "e04".to_string(),
            "f05".to_string(),
            "g06".to_string(),
            "h07".to_string(),
            "i08".to_string(),
            "j09".to_string(),
        ];
        let uids = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let update_result = core
            .update_uids(mailbox_2.clone(), jmap_ids.clone())
            .await
            .unwrap();
        assert_eq!(update_result.uid_next, 11);

        core.purge_deleted_mailboxes(&Account {
            account_id: "john".to_string(),
            prefix: None,
            mailbox_names: BTreeMap::new(),
            mailbox_data: AHashMap::from_iter([("folder_id".to_string(), Mailbox::default())]),
            mailbox_state: String::new(),
            modseq: None,
        })
        .await
        .unwrap();
        let (uid_validity, uid_next) = core.uids(mailbox.clone()).await.unwrap();
        assert_ne!(uid_validity, 0);
        assert_eq!(uid_next, 1);

        assert_eq!(
            core.imap_to_jmap(mailbox_2.clone(), uids.clone())
                .await
                .unwrap()
                .0,
            jmap_ids
        );
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10])
                .await
                .unwrap()
                .0,
            Vec::<String>::new()
        );

        // Delete temporary directory
        if temp_dir.exists() {
            std::fs::remove_dir_all(&temp_dir).unwrap();
        }
    }
}
