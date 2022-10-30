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

use std::{borrow::Cow, sync::Arc};

use ahash::AHashMap;
use jmap_client::email::{self, Header, Property};
use mail_parser::{GetHeader, Message, PartType, RfcHeader};
use tracing::debug;

use crate::{
    core::{
        client::{SelectedMailbox, Session, SessionData},
        message::MappingOptions,
        receiver::Request,
        Command, Flag, IntoStatusResponse, ResponseCode, StatusResponse,
    },
    parser::PushUnique,
    protocol::{
        expunge::Vanished,
        fetch::{
            self, Arguments, Attribute, BodyContents, BodyPart, BodyPartExtension, BodyPartFields,
            DataItem, Envelope, FetchItem, Section,
        },
    },
};

impl Session {
    pub async fn handle_fetch(
        &mut self,
        request: Request<Command>,
        is_uid: bool,
    ) -> Result<(), ()> {
        match request.parse_fetch() {
            Ok(arguments) => {
                let (data, mailbox) = self.state.select_data();
                let is_qresync = self.is_qresync;

                let enabled_condstore = if !self.is_condstore && arguments.changed_since.is_some()
                    || arguments.attributes.contains(&Attribute::ModSeq)
                {
                    self.is_condstore = true;
                    true
                } else {
                    false
                };

                tokio::spawn(async move {
                    data.write_bytes(
                        data.fetch(arguments, mailbox, is_uid, is_qresync, enabled_condstore)
                            .await
                            .into_bytes(),
                    )
                    .await;
                });
                Ok(())
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}

impl SessionData {
    pub async fn fetch(
        &self,
        mut arguments: Arguments,
        mailbox: Arc<SelectedMailbox>,
        is_uid: bool,
        is_qresync: bool,
        mut enabled_condstore: bool,
    ) -> StatusResponse {
        // Validate VANISHED parameter
        if arguments.include_vanished {
            if !is_qresync {
                return StatusResponse::bad("Enable QRESYNC first to use the VANISHED parameter.")
                    .with_tag(arguments.tag);
            } else if !is_uid {
                return StatusResponse::bad("VANISHED parameter is only available for UID FETCH.")
                    .with_tag(arguments.tag);
            }
        }

        // Convert IMAP ids to JMAP ids.
        let mut ids = match mailbox
            .sequence_to_jmap(&arguments.sequence_set, is_uid)
            .await
        {
            Ok(ids) => ids,
            Err(response) => {
                return response.with_tag(arguments.tag);
            }
        };

        // Convert state to modseq
        if let Some(changed_since) = arguments.changed_since {
            // Convert MODSEQ to JMAP State
            let state = match self
                .core
                .modseq_to_state(&mailbox.id.account_id, changed_since as u32)
                .await
            {
                Ok(Some(state)) => state,
                Ok(None) => {
                    return StatusResponse::bad(format!(
                        "MODSEQ '{}' does not exist.",
                        changed_since
                    ))
                    .with_tag(arguments.tag);
                }
                Err(_) => return StatusResponse::database_failure().with_tag(arguments.tag),
            };

            // Obtain changes since the modseq.
            let mut request = self.client.build();
            request
                .changes_email(state)
                .account_id(&mailbox.id.account_id);
            match request.send_changes_email().await {
                Ok(mut changes) => {
                    // Condstore was just enabled, return higest modseq.
                    if enabled_condstore {
                        if let Ok(modseq) = self
                            .core
                            .state_to_modseq(&mailbox.id.account_id, changes.take_new_state())
                            .await
                        {
                            self.write_bytes(
                                StatusResponse::ok("Highest Modseq")
                                    .with_code(ResponseCode::HighestModseq { modseq })
                                    .into_bytes(),
                            )
                            .await;
                            enabled_condstore = false;
                        } else {
                            return StatusResponse::database_failure().with_tag(arguments.tag);
                        }
                    }

                    // Send vanished UIDs
                    if arguments.include_vanished {
                        // Add to vanished all known destroyed Ids
                        if !changes.destroyed().is_empty() || !changes.updated().is_empty() {
                            let mut destroyed_ids = changes.take_destroyed();
                            if !changes.updated().is_empty() {
                                destroyed_ids.extend(changes.updated().iter().cloned());
                            }
                            if let Ok((_, mut vanished)) = self
                                .core
                                .jmap_to_imap(
                                    mailbox.id.clone(),
                                    destroyed_ids,
                                    MappingOptions::OnlyIncludeDeleted,
                                )
                                .await
                            {
                                if !vanished.is_empty() {
                                    vanished.sort_unstable();
                                    let mut buf = Vec::with_capacity(vanished.len() * 3);
                                    Vanished {
                                        earlier: true,
                                        ids: vanished,
                                    }
                                    .serialize(&mut buf);
                                    self.write_bytes(buf).await;
                                }
                            } else {
                                debug!("Failed to convert destroyed ids to IMAP ids");
                            }
                        }
                    }

                    // Filter out ids without changes
                    if changes.created().is_empty() && changes.updated().is_empty() {
                        return StatusResponse::completed(Command::Fetch(is_uid))
                            .with_tag(arguments.tag);
                    }
                    let mut changed_ids =
                        AHashMap::with_capacity(changes.created().len() + changes.updated().len());
                    for jmap_id in changes
                        .take_created()
                        .into_iter()
                        .chain(changes.take_updated())
                    {
                        if let Some(imap_ids) = ids.remove(&jmap_id) {
                            changed_ids.insert(jmap_id, imap_ids);
                        }
                    }
                    if changed_ids.is_empty() {
                        return StatusResponse::completed(Command::Fetch(is_uid))
                            .with_tag(arguments.tag);
                    }
                    ids = changed_ids;
                }
                Err(err) => {
                    return err.into_status_response().with_tag(arguments.tag);
                }
            }

            arguments.attributes.push_unique(Attribute::ModSeq);
        }

        // Build properties list
        let mut properties = Vec::with_capacity(arguments.attributes.len());
        let mut set_seen_flags = false;
        let mut needs_blobs = false;
        let mut needs_modseq = false;
        properties.push(Property::Id);

        for attribute in &arguments.attributes {
            match attribute {
                Attribute::Envelope => {
                    properties.extend([
                        Property::SentAt,
                        Property::Subject,
                        Property::From,
                        Property::Sender,
                        Property::ReplyTo,
                        Property::Header(Header::as_grouped_addresses("To", true)),
                        Property::Header(Header::as_grouped_addresses("Cc", true)),
                        Property::Header(Header::as_grouped_addresses("Bcc", true)),
                        Property::InReplyTo,
                        Property::MessageId,
                    ]);
                }
                Attribute::Flags => {
                    properties.push_unique(Property::Keywords);
                }
                Attribute::InternalDate => {
                    properties.push(Property::ReceivedAt);
                }
                Attribute::Preview { .. } => {
                    properties.push_unique(Property::Preview);
                }
                Attribute::Rfc822Size => {
                    properties.push(Property::Size);
                }
                Attribute::Rfc822Header
                | Attribute::Body
                | Attribute::BodyStructure
                | Attribute::BinarySize { .. } => {
                    /*
                        Note that this did not result in \Seen being set, because
                        RFC822.HEADER response data occurs as a result of a FETCH
                        of RFC822.HEADER.  BODY[HEADER] response data occurs as a
                        result of a FETCH of BODY[HEADER] (which sets \Seen) or
                        BODY.PEEK[HEADER] (which does not set \Seen).
                    */
                    needs_blobs = true;
                    properties.push_unique(Property::BlobId);
                }
                Attribute::BodySection { peek, .. } | Attribute::Binary { peek, .. } => {
                    if mailbox.is_select && !*peek {
                        set_seen_flags = true;
                    }
                    needs_blobs = true;
                    properties.push_unique(Property::BlobId);
                }
                Attribute::Rfc822Text | Attribute::Rfc822 => {
                    if mailbox.is_select {
                        set_seen_flags = true;
                    }
                    needs_blobs = true;
                    properties.push_unique(Property::BlobId);
                }
                Attribute::Uid | Attribute::EmailId => (),
                Attribute::ModSeq => {
                    needs_modseq = true;
                }
                Attribute::ThreadId => {
                    properties.push_unique(Property::ThreadId);
                }
            }
        }
        if set_seen_flags {
            properties.push_unique(Property::Keywords);
        }
        if is_uid {
            arguments.attributes.push_unique(Attribute::Uid);
        }

        // Send request to JMAP server
        let max_objects_in_get = self
            .client
            .session()
            .core_capabilities()
            .map(|c| c.max_objects_in_get())
            .unwrap_or(500);
        let mut modseq = u32::MAX;

        let mut set_seen_ids = Vec::new();
        let ids_vec = ids.keys().collect::<Vec<_>>();
        for jmap_ids in ids_vec.chunks(max_objects_in_get) {
            let mut request = self.client.build();
            request
                .get_email()
                .account_id(&mailbox.id.account_id)
                .ids(jmap_ids.iter().cloned())
                .properties(properties.clone());
            let mut response = match request.send_get_email().await {
                Ok(response) => response,
                Err(response) => {
                    return response.into_status_response().with_tag(arguments.tag);
                }
            };

            // Obtain modseq
            if needs_modseq && modseq == u32::MAX {
                modseq = self
                    .core
                    .state_to_modseq(&mailbox.id.account_id, response.take_state())
                    .await
                    .unwrap_or(u32::MAX)
            }

            // Process each message
            for mut email in response.take_list() {
                // Obtain result position
                let (uid, seqnum) = if let Some(imap_id) = ids.get(email.id().unwrap_or("")) {
                    (imap_id.uid, imap_id.seqnum)
                } else {
                    debug!(
                        "JMAP server returned unexpected email Id {:?}, account {:?}",
                        email.id().unwrap_or(""),
                        mailbox.id.account_id
                    );
                    continue;
                };

                // Fetch and parse blob
                let raw_message = if needs_blobs {
                    match email.blob_id() {
                        Some(blob_id) => match self.client.download(blob_id).await {
                            Ok(raw_message) => raw_message.into(),
                            Err(err) => {
                                debug!(
                                    "Failed to download blob for email Id {:?}, account {:?}: {}",
                                    email.id().unwrap_or(""),
                                    mailbox.id.account_id,
                                    err
                                );
                                continue;
                            }
                        },
                        None => {
                            debug!(
                                "JMAP server returned missing blobId for email Id {:?}, account {:?}",
                                email.id().unwrap_or(""),
                                mailbox.id.account_id,
                            );
                            continue;
                        }
                    }
                } else {
                    None
                };
                let message = if let Some(raw_message) = &raw_message {
                    if let Some(message) = Message::parse(raw_message) {
                        message.into()
                    } else {
                        debug!(
                            "Failed to parse email Id {:?}, account {:?}",
                            email.id().unwrap_or(""),
                            mailbox.id.account_id
                        );
                        continue;
                    }
                } else {
                    None
                };

                // Build response
                let mut items = Vec::with_capacity(arguments.attributes.len());
                let set_seen_flag =
                    set_seen_flags && !email.keywords().iter().any(|&k| k == Flag::Seen.to_jmap());
                for attribute in &arguments.attributes {
                    match attribute {
                        Attribute::Envelope => {
                            items.push(DataItem::Envelope {
                                envelope: Envelope {
                                    date: email.sent_at(),
                                    subject: email.subject().map(|s| s.into()),
                                    from: email
                                        .from()
                                        .map(|addrs| addrs.iter().map(|addr| addr.into()).collect())
                                        .unwrap_or_default(),
                                    sender: email
                                        .sender()
                                        .map(|addrs| addrs.iter().map(|addr| addr.into()).collect())
                                        .unwrap_or_default(),
                                    reply_to: email
                                        .reply_to()
                                        .map(|addrs| addrs.iter().map(|addr| addr.into()).collect())
                                        .unwrap_or_default(),
                                    to: email
                                        .header(&Header::as_grouped_addresses("To", true))
                                        .map(|value| value.as_imap_address())
                                        .unwrap_or_default(),
                                    cc: email
                                        .header(&Header::as_grouped_addresses("Cc", true))
                                        .map(|value| value.as_imap_address())
                                        .unwrap_or_default(),
                                    bcc: email
                                        .header(&Header::as_grouped_addresses("Bcc", true))
                                        .map(|value| value.as_imap_address())
                                        .unwrap_or_default(),
                                    in_reply_to: email.in_reply_to().map(|list| {
                                        let mut irt = String::with_capacity(list.len() * 10);
                                        for (pos, l) in list.iter().enumerate() {
                                            if pos > 0 {
                                                irt.push(' ');
                                            }
                                            irt.push('<');
                                            irt.push_str(l.as_ref());
                                            irt.push('>');
                                        }
                                        irt.into()
                                    }),
                                    message_id: email.message_id().map(|list| {
                                        let mut irt = String::with_capacity(list.len() * 10);
                                        for (pos, l) in list.iter().enumerate() {
                                            if pos > 0 {
                                                irt.push(' ');
                                            }
                                            irt.push('<');
                                            irt.push_str(l.as_ref());
                                            irt.push('>');
                                        }
                                        irt.into()
                                    }),
                                },
                            });
                        }
                        Attribute::Flags => {
                            let mut flags = email
                                .keywords()
                                .iter()
                                .map(|k| Flag::parse_jmap(k.to_string()))
                                .collect::<Vec<_>>();
                            if set_seen_flag {
                                flags.push(Flag::Seen);
                            }
                            items.push(DataItem::Flags { flags });
                        }
                        Attribute::InternalDate => {
                            if let Some(date) = email.received_at() {
                                items.push(DataItem::InternalDate { date });
                            }
                        }
                        Attribute::Preview { .. } => {
                            items.push(DataItem::Preview {
                                contents: email.preview().map(|p| p.into()),
                            });
                        }
                        Attribute::Rfc822Size => {
                            items.push(DataItem::Rfc822Size { size: email.size() });
                        }
                        Attribute::Uid => {
                            items.push(DataItem::Uid { uid });
                        }
                        Attribute::Rfc822 => {
                            items.push(DataItem::Rfc822 {
                                contents: String::from_utf8_lossy(raw_message.as_ref().unwrap()),
                            });
                        }
                        Attribute::Rfc822Header => {
                            let message = message.as_ref().unwrap().get_root_part();
                            if let Some(header) = raw_message
                                .as_ref()
                                .unwrap()
                                .get(message.offset_header..message.offset_body)
                            {
                                items.push(DataItem::Rfc822Header {
                                    contents: String::from_utf8_lossy(header),
                                });
                            }
                        }
                        Attribute::Rfc822Text => {
                            let message = message.as_ref().unwrap().get_root_part();
                            if let Some(text) = raw_message
                                .as_ref()
                                .unwrap()
                                .get(message.offset_body..message.offset_end)
                            {
                                items.push(DataItem::Rfc822Text {
                                    contents: String::from_utf8_lossy(text),
                                });
                            }
                        }
                        Attribute::Body => {
                            items.push(DataItem::Body {
                                part: message.as_ref().unwrap().body_structure(false),
                            });
                        }
                        Attribute::BodyStructure => {
                            items.push(DataItem::BodyStructure {
                                part: message.as_ref().unwrap().body_structure(true),
                            });
                        }
                        Attribute::BodySection {
                            sections, partial, ..
                        } => {
                            if let Some(contents) =
                                message.as_ref().unwrap().body_section(sections, *partial)
                            {
                                items.push(DataItem::BodySection {
                                    sections: sections.to_vec(),
                                    origin_octet: partial.map(|(start, _)| start),
                                    contents,
                                });
                            }
                        }

                        Attribute::Binary {
                            sections, partial, ..
                        } => match message.as_ref().unwrap().binary(sections, *partial) {
                            Ok(Some(contents)) => {
                                items.push(DataItem::Binary {
                                    sections: sections.to_vec(),
                                    offset: partial.map(|(start, _)| start),
                                    contents,
                                });
                            }
                            Err(_) => {
                                self.write_bytes(
                                    StatusResponse::no(format!(
                                        "Failed to decode part {} of message {}.",
                                        sections
                                            .iter()
                                            .map(|s| s.to_string())
                                            .collect::<Vec<_>>()
                                            .join("."),
                                        if is_uid { uid } else { seqnum }
                                    ))
                                    .with_code(ResponseCode::UnknownCte)
                                    .into_bytes(),
                                )
                                .await;
                                continue;
                            }
                            _ => (),
                        },
                        Attribute::BinarySize { sections } => {
                            if let Some(size) = message.as_ref().unwrap().binary_size(sections) {
                                items.push(DataItem::BinarySize {
                                    sections: sections.to_vec(),
                                    size,
                                });
                            }
                        }
                        Attribute::ModSeq => {
                            if modseq != u32::MAX {
                                items.push(DataItem::ModSeq { modseq });
                            }
                        }
                        Attribute::EmailId => {
                            if let Some(email_id) = email.id() {
                                items.push(DataItem::EmailId {
                                    email_id: format!("{}-{}", mailbox.id.account_id, email_id),
                                });
                            }
                        }
                        Attribute::ThreadId => {
                            if let Some(thread_id) = email.thread_id() {
                                items.push(DataItem::ThreadId {
                                    thread_id: format!("{}-{}", mailbox.id.account_id, thread_id),
                                });
                            }
                        }
                    }
                }

                // Add flags to the response if the message was unseen
                if set_seen_flag && !arguments.attributes.contains(&Attribute::Flags) {
                    let mut flags = email
                        .keywords()
                        .iter()
                        .map(|k| Flag::parse_jmap(k.to_string()))
                        .collect::<Vec<_>>();
                    flags.push(Flag::Seen);
                    items.push(DataItem::Flags { flags });
                }

                // Serialize fetch item
                let mut buf = Vec::with_capacity(128);
                FetchItem { id: seqnum, items }.serialize(&mut buf);
                if !self.write_bytes(buf).await {
                    return StatusResponse::completed(Command::Fetch(is_uid))
                        .with_tag(arguments.tag);
                }

                // Add to set flags
                if set_seen_flag {
                    set_seen_ids.push(email.take_id());
                }
            }
        }

        // Set Seen ids
        if !set_seen_ids.is_empty() {
            let max_objects_in_set = self
                .client
                .session()
                .core_capabilities()
                .map(|c| c.max_objects_in_set())
                .unwrap_or(500);

            let mut request = self.client.build();
            for set_seen_ids in set_seen_ids.chunks(max_objects_in_set) {
                let set_request = request.set_email().account_id(&mailbox.id.account_id);
                for set_seen_id in set_seen_ids {
                    set_request
                        .update(set_seen_id)
                        .keyword(Flag::Seen.to_jmap(), true);
                }
            }

            match request.send().await {
                Ok(responses) => {
                    for response in responses.unwrap_method_responses() {
                        match response.unwrap_set_email() {
                            Ok(mut response) => {
                                if enabled_condstore {
                                    modseq = self
                                        .core
                                        .state_to_modseq(
                                            &mailbox.id.account_id,
                                            response.take_new_state(),
                                        )
                                        .await
                                        .unwrap_or(u32::MAX)
                                }
                            }
                            Err(err) => {
                                debug!("Failed to set Seen flags: {}", err);
                                return err.into_status_response().with_tag(arguments.tag);
                            }
                        }
                    }
                }
                Err(err) => {
                    debug!("Failed to set Seen flags: {}", err);
                    return err.into_status_response().with_tag(arguments.tag);
                }
            }
        }

        // Condstore was enabled with this command
        if enabled_condstore {
            if modseq == u32::MAX {
                if let Ok(modseq_) = self.synchronize_state(&mailbox.id.account_id).await {
                    modseq = modseq_;
                }
            }
            if modseq != u32::MAX {
                self.write_bytes(
                    StatusResponse::ok("Highest Modseq")
                        .with_code(ResponseCode::HighestModseq { modseq })
                        .into_bytes(),
                )
                .await;
            }
        }

        StatusResponse::completed(Command::Fetch(is_uid)).with_tag(arguments.tag)
    }
}

trait AsImapDataItem<'x> {
    fn body_structure(&self, is_extended: bool) -> BodyPart;
    fn body_section<'z: 'x>(
        &'z self,
        sections: &[Section],
        partial: Option<(u32, u32)>,
    ) -> Option<Cow<'x, str>>;
    fn binary(
        &self,
        sections: &[u32],
        partial: Option<(u32, u32)>,
    ) -> Result<Option<BodyContents>, ()>;
    fn binary_size(&self, sections: &[u32]) -> Option<usize>;
    fn as_body_part(&self, part_id: usize, is_extended: bool) -> BodyPart;
    fn envelope(&self) -> Envelope;
}

impl<'x> AsImapDataItem<'x> for Message<'x> {
    fn body_structure(&self, is_extended: bool) -> BodyPart {
        let mut stack = Vec::new();
        let mut parts = [0].iter();
        let mut message = self;
        let mut root_part = None;

        loop {
            while let Some(part_id) = parts.next() {
                let mut part = message.as_body_part(*part_id, is_extended);

                match &message.parts[*part_id].body {
                    PartType::Message(nested_message) => {
                        part.set_envelope(nested_message.envelope());
                        if let Some(root_part) = root_part {
                            stack.push((root_part, parts, message.into()));
                        }
                        root_part = part.into();
                        parts = [0].iter();
                        message = nested_message;
                        continue;
                    }
                    PartType::Multipart(subparts) => {
                        if let Some(root_part) = root_part {
                            stack.push((root_part, parts, None));
                        }
                        root_part = part.into();
                        parts = subparts.iter();
                        continue;
                    }
                    _ => (),
                }
                if let Some(root_part) = &mut root_part {
                    root_part.add_part(part);
                } else {
                    return part;
                }
            }
            if let Some((mut prev_root_part, prev_parts, prev_message)) = stack.pop() {
                if let Some(prev_message) = prev_message {
                    message = prev_message;
                }

                prev_root_part.add_part(root_part.unwrap());
                parts = prev_parts;
                root_part = prev_root_part.into();
            } else {
                break;
            }
        }

        root_part.unwrap()
    }

    fn as_body_part(&self, part_id: usize, is_extended: bool) -> BodyPart {
        let part = &self.parts[part_id];
        let body = self.raw_message.get(part.offset_body..part.offset_end);
        let (is_multipart, is_text) = match &part.body {
            PartType::Text(_) | PartType::Html(_) => (false, true),
            PartType::Multipart(_) => (true, false),
            _ => (false, false),
        };
        let content_type = part
            .headers
            .get_rfc(&RfcHeader::ContentType)
            .and_then(|ct| ct.as_content_type_ref());

        let mut body_md5 = None;
        let mut extension = BodyPartExtension::default();
        let mut fields = BodyPartFields::default();

        if !is_multipart || is_extended {
            fields.body_parameters = content_type.as_ref().and_then(|ct| {
                ct.attributes.as_ref().map(|at| {
                    at.iter()
                        .map(|(h, v)| (h.as_ref().into(), v.as_ref().into()))
                        .collect::<Vec<_>>()
                })
            })
        }

        if !is_multipart {
            fields.body_subtype = content_type
                .as_ref()
                .and_then(|ct| ct.c_subtype.as_ref().map(|cs| cs.as_ref().into()));

            fields.body_id = part
                .headers
                .get_rfc(&RfcHeader::ContentId)
                .and_then(|id| id.as_text_ref().map(|id| format!("<{}>", id).into()));

            fields.body_description = part
                .headers
                .get_rfc(&RfcHeader::ContentDescription)
                .and_then(|ct| ct.as_text_ref().map(|ct| ct.into()));

            fields.body_encoding = part
                .headers
                .get_rfc(&RfcHeader::ContentTransferEncoding)
                .and_then(|ct| ct.as_text_ref().map(|ct| ct.into()));

            fields.body_size_octets = body.as_ref().map(|b| b.len()).unwrap_or(0);

            if is_text {
                if fields.body_subtype.is_none() {
                    fields.body_subtype = Some("plain".into());
                }
                if fields.body_encoding.is_none() {
                    fields.body_encoding = Some("7bit".into());
                }
                if fields.body_parameters.is_none() {
                    fields.body_parameters = Some(vec![("charset".into(), "us-ascii".into())]);
                }
            }
        }

        if is_extended {
            if !is_multipart {
                body_md5 = body
                    .as_ref()
                    .map(|b| format!("{:x}", md5::compute(b)).into());
            }

            extension.body_disposition =
                part.headers
                    .get_rfc(&RfcHeader::ContentDisposition)
                    .map(|cd| {
                        let cd = cd.get_content_type();

                        (
                            cd.c_type.as_ref().into(),
                            cd.attributes
                                .as_ref()
                                .map(|at| {
                                    at.iter()
                                        .map(|(h, v)| (h.as_ref().into(), v.as_ref().into()))
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default(),
                        )
                    });

            extension.body_language =
                part.headers
                    .get_rfc(&RfcHeader::ContentLanguage)
                    .and_then(|hv| {
                        hv.as_text_list()
                            .map(|list| list.into_iter().map(|item| item.into()).collect())
                    });

            extension.body_location = part
                .headers
                .get_rfc(&RfcHeader::ContentLocation)
                .and_then(|ct| ct.as_text_ref().map(|ct| ct.into()));
        }

        match &part.body {
            PartType::Multipart(parts) => BodyPart::Multipart {
                body_parts: Vec::with_capacity(parts.len()),
                body_subtype: content_type
                    .as_ref()
                    .and_then(|ct| ct.c_subtype.as_ref().map(|cs| cs.as_ref().into()))
                    .unwrap_or_else(|| "".into()),
                body_parameters: fields.body_parameters,
                extension,
            },
            PartType::Message(_) => BodyPart::Message {
                fields,
                envelope: None,
                body: None,
                body_size_lines: 0,
                body_md5,
                extension,
            },
            _ => {
                if is_text {
                    BodyPart::Text {
                        fields,
                        body_size_lines: body
                            .as_ref()
                            .map(|b| b.iter().filter(|&&ch| ch == b'\n').count())
                            .unwrap_or(0),
                        body_md5,
                        extension,
                    }
                } else {
                    BodyPart::Basic {
                        body_type: content_type
                            .as_ref()
                            .map(|ct| Cow::from(ct.c_type.as_ref())),
                        fields,
                        body_md5,
                        extension,
                    }
                }
            }
        }
    }

    fn body_section<'z: 'x>(
        &'z self,
        sections: &[Section],
        partial: Option<(u32, u32)>,
    ) -> Option<Cow<'x, str>> {
        let mut part = self.get_root_part();
        if sections.is_empty() {
            return String::from_utf8_lossy(get_partial_bytes(
                self.raw_message.get(part.offset_header..part.offset_end)?,
                partial,
            ))
            .into();
        }

        let mut message = self;
        let sections_single = sections.len() == 1;
        let mut sections_iter = sections.iter().peekable();

        while let Some(section) = sections_iter.next() {
            match section {
                Section::Part { num } => {
                    part = if let Some(sub_part_ids) = part.get_sub_parts() {
                        sub_part_ids
                            .get((*num).saturating_sub(1) as usize)
                            .and_then(|pos| message.parts.get(*pos))
                    } else if *num == 1 && (sections_single || part.is_message()) {
                        Some(part)
                    } else {
                        None
                    }?;

                    if let (PartType::Message(nested_message), Some(_)) =
                        (&part.body, sections_iter.peek())
                    {
                        message = nested_message;
                        part = message.get_root_part();
                    }
                }
                Section::Header => {
                    return String::from_utf8_lossy(get_partial_bytes(
                        message
                            .raw_message
                            .get(part.offset_header..part.offset_body)?,
                        partial,
                    ))
                    .into();
                }
                Section::HeaderFields { not, fields } => {
                    let mut headers =
                        Vec::with_capacity(part.offset_body.saturating_sub(part.offset_header));
                    for header in &part.headers {
                        let header_name = header.name.as_str();
                        if fields.iter().any(|f| header_name.eq_ignore_ascii_case(f)) != *not {
                            headers.extend_from_slice(header_name.as_bytes());
                            headers.push(b':');
                            headers.extend_from_slice(
                                message
                                    .raw_message
                                    .get(header.offset_start..header.offset_end)
                                    .unwrap_or(b""),
                            );
                        }
                    }

                    headers.extend_from_slice(b"\r\n");

                    return Some(if partial.is_none() {
                        String::from_utf8(headers).map_or_else(
                            |err| String::from_utf8_lossy(err.as_bytes()).into_owned().into(),
                            |s| s.into(),
                        )
                    } else {
                        String::from_utf8_lossy(get_partial_bytes(&headers, partial))
                            .into_owned()
                            .into()
                    });
                }
                Section::Text => {
                    return String::from_utf8_lossy(get_partial_bytes(
                        message.raw_message.get(part.offset_body..part.offset_end)?,
                        partial,
                    ))
                    .into();
                }
                Section::Mime => {
                    let mut headers =
                        Vec::with_capacity(part.offset_body.saturating_sub(part.offset_header));
                    for header in &part.headers {
                        if header.name.is_mime_header() {
                            headers.extend_from_slice(header.name.as_str().as_bytes());
                            headers.extend_from_slice(b":");
                            headers.extend_from_slice(
                                message
                                    .raw_message
                                    .get(header.offset_start..header.offset_end)
                                    .unwrap_or(b""),
                            );
                        }
                    }
                    headers.extend_from_slice(b"\r\n");
                    return Some(if partial.is_none() {
                        String::from_utf8(headers).map_or_else(
                            |err| String::from_utf8_lossy(err.as_bytes()).into_owned().into(),
                            |s| s.into(),
                        )
                    } else {
                        String::from_utf8_lossy(get_partial_bytes(&headers, partial))
                            .into_owned()
                            .into()
                    });
                }
            }
        }

        // BODY[x] should return both headers and body, but most clients
        // expect BODY[x] to return only the body, just like BOXY[x.TEXT] does.

        String::from_utf8_lossy(get_partial_bytes(
            message.raw_message.get(part.offset_body..part.offset_end)?,
            partial,
        ))
        .into()

        /*String::from_utf8_lossy(get_partial_bytes(
            raw_message.get(part.offset_header..part.offset_end)?,
            partial,
        ))
        .into()*/
    }

    fn binary(
        &self,
        sections: &[u32],
        partial: Option<(u32, u32)>,
    ) -> Result<Option<BodyContents>, ()> {
        let mut message = self;
        let mut part = self.get_root_part();
        let mut sections_iter = sections.iter().peekable();

        while let Some(section) = sections_iter.next() {
            part = if let Some(part) = part
                .get_sub_parts()
                .and_then(|p| p.get((*section).saturating_sub(1) as usize))
                .and_then(|p| message.parts.get(*p))
            {
                part
            } else {
                return Ok(None);
            };
            if let (PartType::Message(nested_message), Some(_)) = (&part.body, sections_iter.peek())
            {
                message = nested_message;
                part = message.get_root_part();
            }
        }

        if !part.is_encoding_problem {
            Ok(match &part.body {
                PartType::Text(text) | PartType::Html(text) => BodyContents::Text(
                    String::from_utf8_lossy(get_partial_bytes(text.as_bytes(), partial)),
                )
                .into(),
                PartType::Binary(bytes) | PartType::InlineBinary(bytes) => {
                    BodyContents::Bytes(get_partial_bytes(bytes.as_ref(), partial).into()).into()
                }
                PartType::Message(message) => BodyContents::Bytes(
                    get_partial_bytes(
                        message
                            .raw_message
                            .get(
                                message.get_root_part().raw_header_offset()
                                    ..message.get_root_part().raw_end_offset(),
                            )
                            .unwrap_or(&b""[..]),
                        partial,
                    )
                    .into(),
                )
                .into(),
                PartType::Multipart(_) => None,
            })
        } else {
            Err(())
        }
    }

    fn binary_size(&self, sections: &[u32]) -> Option<usize> {
        let mut message = self;
        let mut part = self.get_root_part();
        let mut sections_iter = sections.iter().peekable();

        while let Some(section) = sections_iter.next() {
            part = message.parts.get(
                *part
                    .get_sub_parts()?
                    .get((*section).saturating_sub(1) as usize)?,
            )?;
            if let (PartType::Message(nested_message), Some(_)) = (&part.body, sections_iter.peek())
            {
                message = nested_message;
                part = message.get_root_part();
            }
        }

        match &part.body {
            PartType::Text(text) | PartType::Html(text) => text.len(),
            PartType::Binary(bytes) | PartType::InlineBinary(bytes) => bytes.len(),
            PartType::Message(message) => message.get_root_part().raw_len(),
            PartType::Multipart(_) => 0,
        }
        .into()
    }

    fn envelope(&self) -> Envelope {
        Envelope {
            date: self.get_date().map(|dt| dt.to_timestamp()),
            subject: self.get_subject().map(|s| s.into()),
            from: self
                .get_header_values(RfcHeader::From)
                .flat_map(|a| a.as_imap_address())
                .collect(),
            sender: self
                .get_header_values(RfcHeader::Sender)
                .flat_map(|a| a.as_imap_address())
                .collect(),
            reply_to: self
                .get_header_values(RfcHeader::ReplyTo)
                .flat_map(|a| a.as_imap_address())
                .collect(),
            to: self
                .get_header_values(RfcHeader::To)
                .flat_map(|a| a.as_imap_address())
                .collect(),
            cc: self
                .get_header_values(RfcHeader::Cc)
                .flat_map(|a| a.as_imap_address())
                .collect(),
            bcc: self
                .get_header_values(RfcHeader::Bcc)
                .flat_map(|a| a.as_imap_address())
                .collect(),
            in_reply_to: self.get_in_reply_to().as_text_list().map(|list| {
                let mut irt = String::with_capacity(list.len() * 10);
                for (pos, l) in list.iter().enumerate() {
                    if pos > 0 {
                        irt.push(' ');
                    }
                    irt.push('<');
                    irt.push_str(l.as_ref());
                    irt.push('>');
                }
                irt.into()
            }),
            message_id: self.get_message_id().map(|id| format!("<{}>", id).into()),
        }
    }
}

#[inline(always)]
fn get_partial_bytes(bytes: &[u8], partial: Option<(u32, u32)>) -> &[u8] {
    if let Some((start, end)) = partial {
        if let Some(bytes) =
            bytes.get(start as usize..std::cmp::min((start + end) as usize, bytes.len()))
        {
            bytes
        } else {
            &[]
        }
    } else {
        bytes
    }
}

impl<'x> From<&'x email::EmailAddress> for fetch::Address<'x> {
    fn from(email: &'x email::EmailAddress) -> Self {
        fetch::Address::Single(fetch::EmailAddress {
            name: email.name().map(|n| n.into()),
            address: email.email().into(),
        })
    }
}

impl<'x> From<&'x email::EmailAddressGroup> for fetch::Address<'x> {
    fn from(group: &'x email::EmailAddressGroup) -> Self {
        fetch::Address::Group(fetch::AddressGroup {
            name: group.name().map(|n| n.into()),
            addresses: group
                .addresses()
                .iter()
                .map(|email| fetch::EmailAddress {
                    name: email.name().map(|n| n.into()),
                    address: email.email().into(),
                })
                .collect(),
        })
    }
}

trait AsImapAddress {
    fn as_imap_address(&self) -> Vec<fetch::Address>;
}

impl AsImapAddress for email::HeaderValue {
    fn as_imap_address(&self) -> Vec<fetch::Address> {
        match self {
            email::HeaderValue::AsAddressesAll(addrs) => {
                addrs.iter().flatten().map(|addr| addr.into()).collect()
            }
            email::HeaderValue::AsAddresses(addrs) => {
                addrs.iter().map(|addr| addr.into()).collect()
            }
            email::HeaderValue::AsGroupedAddressesAll(groups) => {
                let mut addresses = Vec::with_capacity(groups.len());
                for group in groups.iter().flatten() {
                    if group.name().is_none() {
                        addresses.extend(group.addresses().iter().map(|addr| addr.into()));
                    } else {
                        addresses.push(group.into());
                    }
                }
                addresses
            }
            email::HeaderValue::AsGroupedAddresses(groups) => {
                let mut addresses = Vec::with_capacity(groups.len());
                for group in groups {
                    if group.name().is_none() {
                        addresses.extend(group.addresses().iter().map(|addr| addr.into()));
                    } else {
                        addresses.push(group.into());
                    }
                }
                addresses
            }
            _ => Vec::new(),
        }
    }
}

impl AsImapAddress for mail_parser::HeaderValue<'_> {
    fn as_imap_address(&self) -> Vec<fetch::Address> {
        let mut addresses = Vec::new();

        match self {
            mail_parser::HeaderValue::Address(addr) => {
                if let Some(email) = &addr.address {
                    addresses.push(fetch::Address::Single(fetch::EmailAddress {
                        name: addr.name.as_ref().map(|n| n.as_ref().into()),
                        address: email.as_ref().into(),
                    }));
                }
            }
            mail_parser::HeaderValue::AddressList(list) => {
                for addr in list {
                    if let Some(email) = &addr.address {
                        addresses.push(fetch::Address::Single(fetch::EmailAddress {
                            name: addr.name.as_ref().map(|n| n.as_ref().into()),
                            address: email.as_ref().into(),
                        }));
                    }
                }
            }
            mail_parser::HeaderValue::Group(group) => {
                addresses.push(fetch::Address::Group(fetch::AddressGroup {
                    name: group.name.as_ref().map(|n| n.as_ref().into()),
                    addresses: group
                        .addresses
                        .iter()
                        .filter_map(|addr| {
                            fetch::EmailAddress {
                                name: addr.name.as_ref().map(|n| n.as_ref().into()),
                                address: addr.address.as_ref()?.as_ref().into(),
                            }
                            .into()
                        })
                        .collect(),
                }));
            }
            mail_parser::HeaderValue::GroupList(list) => {
                for group in list {
                    addresses.push(fetch::Address::Group(fetch::AddressGroup {
                        name: group.name.as_ref().map(|n| n.as_ref().into()),
                        addresses: group
                            .addresses
                            .iter()
                            .filter_map(|addr| {
                                fetch::EmailAddress {
                                    name: addr.name.as_ref().map(|n| n.as_ref().into()),
                                    address: addr.address.as_ref()?.as_ref().into(),
                                }
                                .into()
                            })
                            .collect(),
                    }));
                }
            }
            _ => (),
        }

        addresses
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use mail_parser::Message;

    use crate::{
        core::{ResponseCode, StatusResponse},
        protocol::fetch::{BodyContents, DataItem, Section},
    };

    use super::AsImapDataItem;

    #[test]
    fn body_structure() {
        let mut test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        test_dir.push("src");
        test_dir.push("tests");
        test_dir.push("resources");
        test_dir.push("messages");
        for file_name in fs::read_dir(&test_dir).unwrap() {
            let mut file_name = file_name.as_ref().unwrap().path();
            if file_name.extension().map_or(true, |e| e != "txt") {
                continue;
            }

            let raw_message = fs::read(&file_name).unwrap();
            let message = Message::parse(&raw_message).unwrap();
            let mut buf = Vec::new();

            // Serialize body and bodystructure
            for is_extended in [false, true] {
                let mut buf_ = Vec::new();
                message
                    .body_structure(is_extended)
                    .serialize(&mut buf_, is_extended);
                if is_extended {
                    buf.extend_from_slice(b"BODYSTRUCTURE ");
                } else {
                    buf.extend_from_slice(b"BODY ");
                }

                // Poor man's indentation
                let mut indent_count = 0;
                let mut in_quote = false;
                for ch in buf_ {
                    if ch == b'(' && !in_quote {
                        buf.extend_from_slice(b"(\n");
                        indent_count += 1;
                        for _ in 0..indent_count {
                            buf.extend_from_slice(b"   ");
                        }
                    } else if ch == b')' && !in_quote {
                        buf.push(b'\n');
                        indent_count -= 1;
                        for _ in 0..indent_count {
                            buf.extend_from_slice(b"   ");
                        }
                        buf.push(b')');
                    } else {
                        if ch == b'"' {
                            in_quote = !in_quote;
                        }
                        buf.push(ch);
                    }
                }
                buf.extend_from_slice(b"\n\n");
            }

            // Serialize body parts
            let mut iter = 1..9;
            let mut stack = Vec::new();
            let mut sections = Vec::new();
            loop {
                'inner: while let Some(part_id) = iter.next() {
                    if part_id == 1 {
                        for section in [
                            None,
                            Some(Section::Header),
                            Some(Section::Text),
                            Some(Section::Mime),
                        ] {
                            let mut body_sections = sections
                                .iter()
                                .map(|id| Section::Part { num: *id })
                                .collect::<Vec<_>>();
                            let is_first = if let Some(section) = section {
                                body_sections.push(section);
                                false
                            } else {
                                true
                            };

                            if let Some(contents) = message.body_section(&body_sections, None) {
                                DataItem::BodySection {
                                    sections: body_sections,
                                    origin_octet: None,
                                    contents,
                                }
                                .serialize(&mut buf);

                                if is_first {
                                    match message.binary(&sections, None) {
                                        Ok(Some(contents)) => {
                                            buf.push(b'\n');
                                            DataItem::Binary {
                                                sections: sections.clone(),
                                                offset: None,
                                                contents: match contents {
                                                    BodyContents::Bytes(_) => BodyContents::Text(
                                                        "[binary content]".into(),
                                                    ),
                                                    text => text,
                                                },
                                            }
                                            .serialize(&mut buf);
                                        }
                                        Ok(None) => (),
                                        Err(_) => {
                                            buf.push(b'\n');
                                            buf.extend_from_slice(
                                                &StatusResponse::no(format!(
                                                    "Failed to decode part {} of message {}.",
                                                    sections
                                                        .iter()
                                                        .map(|s| s.to_string())
                                                        .collect::<Vec<_>>()
                                                        .join("."),
                                                    0
                                                ))
                                                .with_code(ResponseCode::UnknownCte)
                                                .serialize(Vec::new()),
                                            );
                                        }
                                    }

                                    if let Some(size) = message.binary_size(&sections) {
                                        buf.push(b'\n');
                                        DataItem::BinarySize {
                                            sections: sections.clone(),
                                            size,
                                        }
                                        .serialize(&mut buf);
                                    }
                                }

                                buf.extend_from_slice(b"\n----------------------------------\n");
                            } else {
                                break 'inner;
                            }
                        }
                    }
                    sections.push(part_id);
                    stack.push(iter);
                    iter = 1..9;
                }
                if let Some(prev_iter) = stack.pop() {
                    sections.pop();
                    iter = prev_iter;
                } else {
                    break;
                }
            }

            // Check header fields and partial sections
            for sections in [
                vec![Section::HeaderFields {
                    not: false,
                    fields: vec!["From".to_string(), "To".to_string()],
                }],
                vec![Section::HeaderFields {
                    not: true,
                    fields: vec!["Subject".to_string(), "Cc".to_string()],
                }],
            ] {
                DataItem::BodySection {
                    contents: message.body_section(&sections, None).unwrap(),
                    sections: sections.clone(),
                    origin_octet: None,
                }
                .serialize(&mut buf);
                buf.extend_from_slice(b"\n----------------------------------\n");
                DataItem::BodySection {
                    contents: message.body_section(&sections, (10, 25).into()).unwrap(),
                    sections,
                    origin_octet: 10.into(),
                }
                .serialize(&mut buf);
                buf.extend_from_slice(b"\n----------------------------------\n");
            }

            file_name.set_extension("imap");

            let expected_result = fs::read(&file_name).unwrap();

            if buf != expected_result {
                file_name.set_extension("imap_failed");
                fs::write(&file_name, buf).unwrap();
                panic!("Failed test, written output to {}", file_name.display());
            }
        }
    }
}
