use std::{sync::Arc, time::SystemTime};

use jmap_client::{
    core::query::{self, Filter},
    email,
};
use tokio::sync::watch;

use crate::{
    core::{
        client::{Session, SessionData},
        message::{IdMappings, MailboxData},
        receiver::Request,
        Command, Flag, IntoStatusResponse, StatusResponse,
    },
    protocol::{
        search::{self, Arguments, Response, ResultOption},
        Sequence,
    },
};

pub enum SavedSearch {
    InFlight {
        rx: watch::Receiver<Arc<IdMappings>>,
    },
    Results {
        items: Arc<IdMappings>,
    },
    None,
}

impl Session {
    pub async fn handle_search(
        &mut self,
        request: Request,
        is_sort: bool,
        is_uid: bool,
    ) -> Result<(), ()> {
        match if !is_sort {
            request.parse_search(self.version)
        } else {
            request.parse_sort()
        } {
            Ok(mut arguments) => {
                let (data, mailbox) = self.state.mailbox_data();

                // Create channel for results
                let (results_tx, prev_saved_search) =
                    if arguments.result_options.contains(&ResultOption::Save) {
                        let prev_saved_search = Some(data.get_saved_search().await);
                        let (tx, rx) = watch::channel(Arc::new(IdMappings::default()));
                        *data.saved_search.lock() = SavedSearch::InFlight { rx };
                        (tx.into(), prev_saved_search)
                    } else {
                        (None, None)
                    };

                tokio::spawn(async move {
                    let tag = std::mem::take(&mut arguments.tag);
                    let bytes = match data
                        .search(
                            arguments,
                            mailbox,
                            results_tx,
                            prev_saved_search.clone(),
                            is_uid,
                        )
                        .await
                    {
                        Ok(response) => {
                            let response = response.serialize(&tag);
                            StatusResponse::completed(if !is_sort {
                                Command::Search(is_uid)
                            } else {
                                Command::Sort(is_uid)
                            })
                            .with_tag(tag)
                            .serialize(response)
                        }
                        Err(response) => {
                            if let Some(prev_saved_search) = prev_saved_search {
                                *data.saved_search.lock() = prev_saved_search
                                    .map_or(SavedSearch::None, |s| SavedSearch::Results {
                                        items: s,
                                    });
                            }
                            response.with_tag(tag).into_bytes()
                        }
                    };
                    data.write_bytes(bytes).await;
                });
                Ok(())
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}

impl SessionData {
    pub async fn search(
        &self,
        arguments: Arguments,
        mailbox: Arc<MailboxData>,
        results_tx: Option<watch::Sender<Arc<IdMappings>>>,
        prev_saved_search: Option<Option<Arc<IdMappings>>>,
        is_uid: bool,
    ) -> Result<search::Response, StatusResponse> {
        // Convert IMAP to JMAP query
        let (filter, highest_modseq) = self
            .imap_filter_to_jmap(arguments.filter, mailbox.clone(), prev_saved_search, is_uid)
            .await?;
        let sort = arguments.sort.map(|sort| {
            sort.into_iter()
                .map(|comp| {
                    match comp.sort {
                        search::Sort::Arrival => email::query::Comparator::received_at(),
                        search::Sort::Cc => email::query::Comparator::cc(),
                        search::Sort::Date => email::query::Comparator::sent_at(),
                        search::Sort::From => email::query::Comparator::from(),
                        search::Sort::DisplayFrom => email::query::Comparator::from(),
                        search::Sort::Size => email::query::Comparator::size(),
                        search::Sort::Subject => email::query::Comparator::subject(),
                        search::Sort::To => email::query::Comparator::to(),
                        search::Sort::DisplayTo => email::query::Comparator::to(),
                    }
                    .is_ascending(comp.ascending)
                })
                .collect::<Vec<_>>()
        });

        // Build query
        let mut jmap_ids = Vec::new();
        let mut total;
        match filter {
            Filter::FilterCondition(email::query::Filter::Id { value })
                if highest_modseq.is_some() && sort.is_none() =>
            {
                total = value.len();
                jmap_ids = value;
            }
            filter => {
                let mut position = 0;
                loop {
                    let mut request = self.client.build();
                    let query_request = request
                        .query_email()
                        .filter(filter.clone())
                        .calculate_total(true)
                        .position(position);
                    if let Some(sort) = &sort {
                        query_request.sort(sort.clone());
                    }
                    let mut response = match request.send_query_email().await {
                        Ok(response) => response,
                        Err(err) => return Err(err.into_status_response()),
                    };
                    total = response.total().unwrap_or(0);
                    let response = response.take_ids();
                    let response_len = response.len();
                    if response_len > 0 {
                        jmap_ids.extend(response);
                        if jmap_ids.len() < total {
                            position += response_len as i32;
                            continue;
                        }
                    }
                    break;
                }
            }
        }

        // Convert to IMAP ids
        let ids = match self
            .core
            .jmap_to_imap(mailbox, jmap_ids, true, is_uid && results_tx.is_none())
            .await
        {
            Ok(ids) => ids,
            Err(_) => return Err(StatusResponse::database_failure()),
        };

        // Calculate min and max
        let min = if arguments.result_options.contains(&ResultOption::Min) {
            (if is_uid {
                ids.uids.as_ref()
            } else {
                ids.seqnums.as_ref().unwrap()
            })
            .iter()
            .max()
            .copied()
        } else {
            None
        };
        let max = if arguments.result_options.contains(&ResultOption::Max) {
            (if is_uid {
                ids.uids.as_ref()
            } else {
                ids.seqnums.as_ref().unwrap()
            })
            .iter()
            .min()
            .copied()
        } else {
            None
        };

        // Build results
        let ids = Arc::new(if min.is_some() && max.is_some() {
            let mut save_ids = IdMappings {
                jmap_ids: Vec::with_capacity(2),
                uids: Vec::with_capacity(2),
                seqnums: Vec::with_capacity(2).into(),
            };
            for min_max in [min, max].into_iter().flatten() {
                if let Some(pos) = (if is_uid {
                    ids.uids.as_ref()
                } else {
                    ids.seqnums.as_ref().unwrap()
                })
                .iter()
                .position(|&id| id == min_max)
                {
                    if let (Some(jmap_id), Some(uid), Some(seqnum)) = (
                        ids.jmap_ids.get(pos),
                        ids.uids.get(pos),
                        ids.seqnums.as_ref().and_then(|ids| ids.get(pos)),
                    ) {
                        save_ids.jmap_ids.push(jmap_id.clone());
                        save_ids.uids.push(*uid);
                        save_ids.seqnums.as_mut().unwrap().push(*seqnum);
                    }
                }
            }
            save_ids
        } else {
            ids
        });

        // Save results
        if let Some(results_tx) = results_tx {
            *self.saved_search.lock() = SavedSearch::Results { items: ids.clone() };
            results_tx.send(ids.clone()).ok();
        }

        // Build response
        Ok(Response {
            is_uid,
            min,
            max,
            count: if arguments.result_options.contains(&ResultOption::Count) {
                Some(total as u32)
            } else {
                None
            },
            ids: if arguments.result_options.is_empty()
                || arguments.result_options.contains(&ResultOption::All)
            {
                let mut ids = if is_uid {
                    ids.uids.clone()
                } else {
                    ids.seqnums.as_ref().unwrap().clone()
                };
                if sort.is_none() {
                    ids.sort_unstable();
                }
                ids
            } else {
                vec![]
            },
            is_sort: sort.is_some(),
            is_esearch: arguments.is_esearch,
            highest_modseq,
        })
    }

    pub async fn imap_filter_to_jmap(
        &self,
        filter: search::Filter,
        mailbox: Arc<MailboxData>,
        prev_saved_search: Option<Option<Arc<IdMappings>>>,
        is_uid: bool,
    ) -> crate::core::Result<(query::Filter<email::query::Filter>, Option<u32>)> {
        let (imap_filters, mut operator) = match filter {
            search::Filter::Operator(operator, filters) => (filters, operator),
            _ => (vec![filter], query::Operator::And),
        };
        let mut stack = Vec::new();
        let mut jmap_filters: Vec<query::Filter<email::query::Filter>> =
            Vec::with_capacity(imap_filters.len() + 1);
        let mut imap_filters = imap_filters.into_iter();

        if let Some(mailbox_id) = &mailbox.mailbox_id {
            jmap_filters.push(email::query::Filter::in_mailbox(mailbox_id.clone()).into());
        }
        let mut seen_modseq = false;
        let mut highest_modseq = None;

        loop {
            while let Some(filter) = imap_filters.next() {
                match filter {
                    search::Filter::Sequence(sequence, uid_filter) => {
                        let ids = match (&sequence, &prev_saved_search) {
                            (Sequence::SavedSearch, Some(prev_saved_search)) => {
                                if let Some(prev_saved_search) = prev_saved_search {
                                    prev_saved_search.clone()
                                } else {
                                    return Err(StatusResponse::no("No saved search found."));
                                }
                            }
                            _ => self
                                .imap_sequence_to_jmap(
                                    mailbox.clone(),
                                    sequence,
                                    if uid_filter { true } else { is_uid },
                                )
                                .await?
                                .clone(),
                        };

                        jmap_filters.push(email::query::Filter::id(ids.jmap_ids.iter()).into());
                    }
                    search::Filter::All => (),
                    search::Filter::From(text) => {
                        jmap_filters.push(email::query::Filter::from(text).into());
                    }
                    search::Filter::To(text) => {
                        jmap_filters.push(email::query::Filter::to(text).into());
                    }
                    search::Filter::Cc(text) => {
                        jmap_filters.push(email::query::Filter::cc(text).into());
                    }
                    search::Filter::Bcc(text) => {
                        jmap_filters.push(email::query::Filter::bcc(text).into());
                    }
                    search::Filter::Body(text) => {
                        jmap_filters.push(email::query::Filter::body(text).into());
                    }
                    search::Filter::Subject(text) => {
                        jmap_filters.push(email::query::Filter::subject(text).into());
                    }
                    search::Filter::Text(text) => {
                        jmap_filters.push(email::query::Filter::text(text).into());
                    }
                    search::Filter::Header(header, value) => {
                        jmap_filters.push(
                            email::query::Filter::header(
                                header,
                                if !value.is_empty() { Some(value) } else { None },
                            )
                            .into(),
                        );
                    }

                    search::Filter::On(date) => {
                        jmap_filters.push(query::Filter::and([
                            email::query::Filter::after(date),
                            email::query::Filter::before(date + 86400),
                        ]));
                    }
                    search::Filter::Since(date) => {
                        jmap_filters.push(email::query::Filter::after(date).into());
                    }
                    search::Filter::Before(date) => {
                        jmap_filters.push(email::query::Filter::before(date).into());
                    }

                    search::Filter::SentOn(date) => {
                        jmap_filters.push(query::Filter::and([
                            email::query::Filter::sent_after(date),
                            email::query::Filter::sent_before(date + 86400),
                        ]));
                    }
                    search::Filter::SentSince(date) => {
                        jmap_filters.push(email::query::Filter::sent_after(date).into());
                    }
                    search::Filter::SentBefore(date) => {
                        jmap_filters.push(email::query::Filter::sent_before(date).into());
                    }

                    search::Filter::Older(date) => {
                        jmap_filters.push(
                            email::query::Filter::after(
                                SystemTime::now()
                                    .duration_since(SystemTime::UNIX_EPOCH)
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0)
                                    .saturating_sub(date as u64)
                                    as i64,
                            )
                            .into(),
                        );
                    }
                    search::Filter::Younger(date) => {
                        jmap_filters.push(
                            email::query::Filter::after(
                                SystemTime::now()
                                    .duration_since(SystemTime::UNIX_EPOCH)
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0)
                                    .saturating_sub(date as u64)
                                    as i64,
                            )
                            .into(),
                        );
                    }

                    search::Filter::Larger(size) => {
                        jmap_filters.push(email::query::Filter::min_size(size).into());
                    }
                    search::Filter::Smaller(size) => {
                        jmap_filters.push(email::query::Filter::max_size(size).into());
                    }

                    search::Filter::Answered => {
                        jmap_filters.push(
                            email::query::Filter::has_keyword(Flag::Answered.to_jmap()).into(),
                        );
                    }
                    search::Filter::Deleted => {
                        jmap_filters.push(
                            email::query::Filter::has_keyword(Flag::Deleted.to_jmap()).into(),
                        );
                    }
                    search::Filter::Draft => {
                        jmap_filters
                            .push(email::query::Filter::has_keyword(Flag::Draft.to_jmap()).into());
                    }
                    search::Filter::Flagged => {
                        jmap_filters.push(
                            email::query::Filter::has_keyword(Flag::Flagged.to_jmap()).into(),
                        );
                    }
                    search::Filter::Keyword(keyword) => {
                        jmap_filters
                            .push(email::query::Filter::has_keyword(keyword.to_jmap()).into());
                    }
                    search::Filter::Seen => {
                        jmap_filters
                            .push(email::query::Filter::has_keyword(Flag::Seen.to_jmap()).into());
                    }
                    search::Filter::Unanswered => {
                        jmap_filters.push(
                            email::query::Filter::not_keyword(Flag::Answered.to_jmap()).into(),
                        );
                    }
                    search::Filter::Undeleted => {
                        jmap_filters.push(
                            email::query::Filter::not_keyword(Flag::Deleted.to_jmap()).into(),
                        );
                    }
                    search::Filter::Undraft => {
                        jmap_filters
                            .push(email::query::Filter::not_keyword(Flag::Draft.to_jmap()).into());
                    }
                    search::Filter::Unflagged => {
                        jmap_filters.push(
                            email::query::Filter::not_keyword(Flag::Flagged.to_jmap()).into(),
                        );
                    }
                    search::Filter::Unkeyword(keyword) => {
                        jmap_filters
                            .push(email::query::Filter::not_keyword(keyword.to_jmap()).into());
                    }
                    search::Filter::Unseen => {
                        jmap_filters
                            .push(email::query::Filter::not_keyword(Flag::Seen.to_jmap()).into());
                    }
                    search::Filter::Recent => {
                        jmap_filters
                            .push(email::query::Filter::has_keyword(Flag::Recent.to_jmap()).into());
                    }
                    search::Filter::New => {
                        jmap_filters.push(query::Filter::and([
                            email::query::Filter::has_keyword(Flag::Recent.to_jmap()),
                            email::query::Filter::not_keyword(Flag::Seen.to_jmap()),
                        ]));
                    }
                    search::Filter::Old => {
                        jmap_filters
                            .push(email::query::Filter::has_keyword(Flag::Recent.to_jmap()).into());
                    }
                    search::Filter::Operator(new_operator, new_imap_filters) => {
                        stack.push((operator, imap_filters, jmap_filters));
                        jmap_filters = Vec::with_capacity(new_imap_filters.len());
                        operator = new_operator;
                        imap_filters = new_imap_filters.into_iter();
                    }
                    search::Filter::ModSeq((modseq, _)) => {
                        if seen_modseq {
                            return Err(StatusResponse::no(
                                "Only one MODSEQ parameter per query is allowed.",
                            ));
                        }
                        // Convert MODSEQ to JMAP State
                        let state = match self
                            .core
                            .modseq_to_state(&mailbox.account_id, modseq as u32)
                            .await
                        {
                            Ok(Some(state)) => state,
                            Ok(None) => {
                                return Err(StatusResponse::bad(format!(
                                    "MODSEQ '{}' does not exist.",
                                    modseq
                                )));
                            }
                            Err(_) => {
                                return Err(StatusResponse::database_failure());
                            }
                        };

                        // Obtain changes since the modseq.
                        let mut request = self.client.build();
                        request.changes_email(state).account_id(&mailbox.account_id);
                        let mut response = request
                            .send_changes_email()
                            .await
                            .map_err(|err| err.into_status_response())?;

                        // Obtain highest modseq
                        highest_modseq = self
                            .core
                            .state_to_modseq(&mailbox.account_id, response.take_new_state())
                            .await
                            .map_err(|_| StatusResponse::database_failure())?
                            .into();

                        seen_modseq = true;
                        jmap_filters.push(
                            email::query::Filter::id(
                                response
                                    .take_updated()
                                    .into_iter()
                                    .chain(response.take_created()),
                            )
                            .into(),
                        );
                    }
                }
            }

            if let Some((prev_operator, prev_imap_filters, mut prev_jmap_filters)) = stack.pop() {
                prev_jmap_filters.push(query::Filter::operator(operator, jmap_filters));
                jmap_filters = prev_jmap_filters;
                operator = prev_operator;
                imap_filters = prev_imap_filters;
            } else {
                break;
            }
        }

        Ok(if jmap_filters.len() == 1 {
            (jmap_filters.pop().unwrap(), highest_modseq)
        } else {
            (
                query::Filter::operator(operator, jmap_filters),
                highest_modseq,
            )
        })
    }

    pub async fn get_saved_search(&self) -> Option<Arc<IdMappings>> {
        let mut rx = match &*self.saved_search.lock() {
            SavedSearch::InFlight { rx } => rx.clone(),
            SavedSearch::Results { items } => {
                return Some(items.clone());
            }
            SavedSearch::None => {
                return None;
            }
        };
        rx.changed().await.ok();
        let v = rx.borrow();
        Some(v.clone())
    }
}

impl SavedSearch {
    pub async fn unwrap(&self) -> Option<Arc<IdMappings>> {
        match self {
            SavedSearch::InFlight { rx } => {
                let mut rx = rx.clone();
                rx.changed().await.ok();
                let v = rx.borrow();
                Some(v.clone())
            }
            SavedSearch::Results { items } => Some(items.clone()),
            SavedSearch::None => None,
        }
    }
}
