use std::{collections::HashSet, fmt::Display};

use jmap_client::core::set::from_timestamp;

use crate::core::{Command, Flag, ResponseCode, ResponseType, StatusResponse};

pub mod acl;
pub mod append;
pub mod authenticate;
pub mod capability;
pub mod copy_move;
pub mod create;
pub mod delete;
pub mod enable;
pub mod expunge;
pub mod fetch;
pub mod list;
pub mod login;
pub mod namespace;
pub mod rename;
pub mod search;
pub mod select;
pub mod status;
pub mod store;
pub mod subscribe;
pub mod thread;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolVersion {
    Rev1,
    Rev2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sequence {
    Number {
        value: u32,
    },
    Range {
        start: Option<u32>,
        end: Option<u32>,
    },
    SavedSearch,
    List {
        items: Vec<Sequence>,
    },
}

impl Sequence {
    pub fn number(value: u32) -> Sequence {
        Sequence::Number { value }
    }

    pub fn range(start: Option<u32>, end: Option<u32>) -> Sequence {
        Sequence::Range { start, end }
    }

    pub fn contains(&self, value: u32) -> bool {
        match self {
            Sequence::Number { value: number } => *number == value,
            Sequence::Range { start, end } => match (start, end) {
                (Some(start), Some(end)) => value >= *start && value <= *end,
                (Some(start), None) => value >= *start,
                (None, Some(end)) => value <= *end,
                (None, None) => true,
            },
            Sequence::List { items } => {
                for item in items {
                    if item.contains(value) {
                        return true;
                    }
                }
                false
            }
            Sequence::SavedSearch => false,
        }
    }

    pub fn try_expand(&self) -> Option<Vec<u32>> {
        match self {
            Sequence::Number { value } => Some(vec![*value]),
            Sequence::List { items } => {
                let mut result = HashSet::with_capacity(items.len());
                for item in items {
                    match item {
                        Sequence::Number { value } => {
                            result.insert(*value);
                        }
                        Sequence::Range {
                            start: Some(start),
                            end: Some(end),
                        } if *end > *start && (*end - *start) < 1000 => {
                            result.extend(*start..=*end);
                        }
                        Sequence::Range {
                            start: None,
                            end: Some(end),
                        } if *end < 1000 => {
                            result.extend(0..=*end);
                        }
                        _ => return None,
                    }
                }
                Some(result.into_iter().collect())
            }
            Sequence::Range {
                start: Some(start),
                end: Some(end),
            } if *end > *start && (*end - *start) < 1000 => Some((*start..=*end).collect()),
            Sequence::Range {
                start: None,
                end: Some(end),
            } if *end < 1000 => Some((0..=*end).collect()),
            _ => None,
        }
    }
}

pub trait ImapResponse {
    fn serialize(self) -> Vec<u8>;
}

pub fn quoted_string(buf: &mut Vec<u8>, text: &str) {
    buf.push(b'"');
    for &c in text.as_bytes() {
        if c == b'\\' || c == b'"' {
            buf.push(b'\\');
        }
        buf.push(c);
    }
    buf.push(b'"');
}

pub fn quoted_string_or_nil(buf: &mut Vec<u8>, text: Option<&str>) {
    if let Some(text) = text {
        quoted_string(buf, text);
    } else {
        buf.extend_from_slice(b"NIL");
    }
}

pub fn literal_string(buf: &mut Vec<u8>, text: &str) {
    buf.push(b'{');
    buf.extend_from_slice(text.len().to_string().as_bytes());
    buf.extend_from_slice(b"}\r\n");
    buf.extend_from_slice(text.as_bytes());
}

pub fn quoted_timestamp(buf: &mut Vec<u8>, timestamp: i64) {
    buf.push(b'"');
    buf.extend_from_slice(from_timestamp(timestamp).to_rfc2822().as_bytes());
    buf.push(b'"');
}

pub fn quoted_timestamp_or_nil(buf: &mut Vec<u8>, timestamp: Option<i64>) {
    if let Some(timestamp) = timestamp {
        quoted_timestamp(buf, timestamp);
    } else {
        buf.extend_from_slice(b"NIL");
    }
}

impl Flag {
    pub fn serialize(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(match self {
            Flag::Seen => b"\\Seen",
            Flag::Draft => b"\\Draft",
            Flag::Flagged => b"\\Flagged",
            Flag::Answered => b"\\Answered",
            Flag::Recent => b"\\Recent",
            Flag::Important => b"\\Important",
            Flag::Phishing => b"$Phishing",
            Flag::Junk => b"$Junk",
            Flag::NotJunk => b"$NotJunk",
            Flag::Deleted => b"\\Deleted",
            Flag::Forwarded => b"$Forwarded",
            Flag::MDNSent => b"$MDNSent",
            Flag::Keyword(keyword) => keyword.as_bytes(),
        });
    }

    pub fn to_jmap(&self) -> &str {
        match self {
            Flag::Seen => "$seen",
            Flag::Draft => "$draft",
            Flag::Flagged => "$flagged",
            Flag::Answered => "$answered",
            Flag::Recent => "$recent",
            Flag::Important => "$important",
            Flag::Phishing => "$phishing",
            Flag::Junk => "$junk",
            Flag::NotJunk => "$notjunk",
            Flag::Deleted => "$deleted",
            Flag::Forwarded => "$forwarded",
            Flag::MDNSent => "$mdnsent",
            Flag::Keyword(keyword) => keyword,
        }
    }
}

impl ResponseCode {
    pub fn serialize(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(match self {
            ResponseCode::Alert => b"ALERT",
            ResponseCode::AlreadyExists => b"ALREADYEXISTS",
            ResponseCode::AppendUid { uid_validity, uids } => {
                buf.extend_from_slice(format!("APPENDUID {} ", uid_validity).as_bytes());
                serialize_sequence(buf, uids);
                return;
            }
            ResponseCode::AuthenticationFailed => b"AUTHENTICATIONFAILED",
            ResponseCode::AuthorizationFailed => b"AUTHORIZATIONFAILED",
            ResponseCode::BadCharset => b"BADCHARSET",
            ResponseCode::Cannot => b"CANNOT",
            ResponseCode::Capability => b"CAPABILITY",
            ResponseCode::ClientBug => b"CLIENTBUG",
            ResponseCode::Closed => b"CLOSED",
            ResponseCode::ContactAdmin => b"CONTACTADMIN",
            ResponseCode::CopyUid { uid_validity, uids } => {
                buf.extend_from_slice(format!("COPYUID {} ", uid_validity).as_bytes());
                serialize_sequence(buf, uids);
                return;
            }
            ResponseCode::Corruption => b"CORRUPTION",
            ResponseCode::Expired => b"EXPIRED",
            ResponseCode::ExpungeIssued => b"EXPUNGEISSUED",
            ResponseCode::HasChildren => b"HASCHILDREN",
            ResponseCode::InUse => b"INUSE",
            ResponseCode::Limit => b"LIMIT",
            ResponseCode::NonExistent => b"NONEXISTENT",
            ResponseCode::NoPerm => b"NOPERM",
            ResponseCode::OverQuota => b"OVERQUOTA",
            ResponseCode::Parse => b"PARSE",
            ResponseCode::PermanentFlags => b"PERMANENTFLAGS",
            ResponseCode::PrivacyRequired => b"PRIVACYREQUIRED",
            ResponseCode::ReadOnly => b"READ-ONLY",
            ResponseCode::ReadWrite => b"READ-WRITE",
            ResponseCode::ServerBug => b"SERVERBUG",
            ResponseCode::TryCreate => b"TRYCREATE",
            ResponseCode::UidNext => b"UIDNEXT",
            ResponseCode::UidNotSticky => b"UIDNOTSTICKY",
            ResponseCode::UidValidity => b"UIDVALIDITY",
            ResponseCode::Unavailable => b"UNAVAILABLE",
            ResponseCode::UnknownCte => b"UNKNOWN-CTE",
            ResponseCode::Modified { ids } => {
                buf.extend_from_slice(b"MODIFIED ");
                serialize_sequence(buf, ids);
                return;
            }
        });
    }
}

impl ResponseType {
    pub fn serialize(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(match self {
            ResponseType::Ok => b"OK",
            ResponseType::No => b"NO",
            ResponseType::Bad => b"BAD",
            ResponseType::PreAuth => b"PREAUTH",
            ResponseType::Bye => b"BYE",
        });
    }
}

impl StatusResponse {
    pub fn serialize(self, mut buf: Vec<u8>) -> Vec<u8> {
        if let Some(tag) = &self.tag {
            buf.extend_from_slice(tag.as_bytes());
        } else {
            buf.push(b'*');
        }
        buf.push(b' ');
        self.rtype.serialize(&mut buf);
        buf.push(b' ');
        if let Some(code) = &self.code {
            buf.push(b'[');
            code.serialize(&mut buf);
            buf.extend_from_slice(b"] ");
        }
        buf.extend_from_slice(self.message.as_bytes());
        buf.extend_from_slice(b"\r\n");
        buf
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.serialize(Vec::with_capacity(16))
    }
}

impl ProtocolVersion {
    #[inline(always)]
    pub fn is_rev2(&self) -> bool {
        matches!(self, ProtocolVersion::Rev2)
    }

    #[inline(always)]
    pub fn is_rev1(&self) -> bool {
        matches!(self, ProtocolVersion::Rev1)
    }
}

pub fn serialize_sequence(buf: &mut Vec<u8>, list: &[u32]) {
    let mut ids = list.iter().peekable();
    while let Some(&id) = ids.next() {
        buf.extend_from_slice(id.to_string().as_bytes());
        let mut range_id = id;
        loop {
            match ids.peek() {
                Some(&&next_id) if next_id == range_id + 1 => {
                    range_id += 1;
                    ids.next();
                }
                next => {
                    if range_id != id {
                        buf.push(b':');
                        buf.extend_from_slice(range_id.to_string().as_bytes());
                    }
                    if next.is_some() {
                        buf.push(b',');
                    }
                    break;
                }
            }
        }
    }
}

impl Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Command::Capability => write!(f, "CAPABILITY"),
            Command::Noop => write!(f, "NOOP"),
            Command::Logout => write!(f, "LOGOUT"),
            Command::StartTls => write!(f, "STARTTLS"),
            Command::Authenticate => write!(f, "AUTHENTICATE"),
            Command::Login => write!(f, "LOGIN"),
            Command::Enable => write!(f, "ENABLE"),
            Command::Select => write!(f, "SELECT"),
            Command::Examine => write!(f, "EXAMINE"),
            Command::Create => write!(f, "CREATE"),
            Command::Delete => write!(f, "DELETE"),
            Command::Rename => write!(f, "RENAME"),
            Command::Subscribe => write!(f, "SUBSCRIBE"),
            Command::Unsubscribe => write!(f, "UNSUBSCRIBE"),
            Command::List => write!(f, "LIST"),
            Command::Namespace => write!(f, "NAMESPACE"),
            Command::Status => write!(f, "STATUS"),
            Command::Append => write!(f, "APPEND"),
            Command::Idle => write!(f, "IDLE"),
            Command::Close => write!(f, "CLOSE"),
            Command::Unselect => write!(f, "UNSELECT"),
            Command::Expunge(false) => write!(f, "EXPUNGE"),
            Command::Search(false) => write!(f, "SEARCH"),
            Command::Fetch(false) => write!(f, "FETCH"),
            Command::Store(false) => write!(f, "STORE"),
            Command::Copy(false) => write!(f, "COPY"),
            Command::Move(false) => write!(f, "MOVE"),
            Command::Sort(false) => write!(f, "SORT"),
            Command::Thread(false) => write!(f, "THREAD"),
            Command::Expunge(true) => write!(f, "UID EXPUNGE"),
            Command::Search(true) => write!(f, "UID SEARCH"),
            Command::Fetch(true) => write!(f, "UID FETCH"),
            Command::Store(true) => write!(f, "UID STORE"),
            Command::Copy(true) => write!(f, "UID COPY"),
            Command::Move(true) => write!(f, "UID MOVE"),
            Command::Sort(true) => write!(f, "UID SORT"),
            Command::Thread(true) => write!(f, "UID THREAD"),
            Command::Lsub => write!(f, "LSUB"),
            Command::Check => write!(f, "CHECK"),
            Command::SetAcl => write!(f, "SETACL"),
            Command::DeleteAcl => write!(f, "DELETEACL"),
            Command::GetAcl => write!(f, "GETACL"),
            Command::ListRights => write!(f, "LISTRIGHTS"),
            Command::MyRights => write!(f, "MYRIGHTS"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::parse_sequence_set;

    #[test]
    fn sequence_set_contains() {
        for (sequence, expected_result) in [
            ("1,5:10", vec![1, 5, 6, 7, 8, 9, 10]),
            ("2,4:7,9,12:*", vec![2, 4, 5, 6, 7, 9, 12, 13, 14, 15]),
            ("*:4,5:7", vec![1, 2, 3, 4, 5, 6, 7]),
            ("2,4,5", vec![2, 4, 5]),
        ] {
            let sequence = parse_sequence_set(sequence.as_bytes()).unwrap();

            assert_eq!(
                (1..=15)
                    .into_iter()
                    .filter(|num| sequence.contains(*num))
                    .collect::<Vec<_>>(),
                expected_result
            );
        }
    }
}
