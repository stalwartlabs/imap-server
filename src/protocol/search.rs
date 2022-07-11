use jmap_client::core::query::Operator;

use crate::core::{Command, Flag, StatusResponse};

use super::{quoted_string, serialize_sequence, ImapResponse, ProtocolVersion, Sequence};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arguments {
    pub tag: String,
    pub result_options: Vec<ResultOption>,
    pub filter: Filter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub is_uid: bool,
    pub ids: Vec<u32>,
    pub min: Option<u32>,
    pub max: Option<u32>,
    pub count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultOption {
    Min,
    Max,
    All,
    Count,
    Save,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    Sequence(Sequence, bool),
    All,
    Answered,
    Bcc(String),
    Before(i64),
    Body(String),
    Cc(String),
    Deleted,
    Draft,
    Flagged,
    From(String),
    Header(String, String),
    Keyword(Flag),
    Larger(u32),
    On(i64),
    Seen,
    SentBefore(i64),
    SentOn(i64),
    SentSince(i64),
    Since(i64),
    Smaller(u32),
    Subject(String),
    Text(String),
    To(String),
    Unanswered,
    Undeleted,
    Undraft,
    Unflagged,
    Unkeyword(Flag),
    Unseen,
    Operator(Operator, Vec<Filter>),

    // Imap4rev1
    Recent,
    New,
    Old,

    // RFC5032
    Older(u32),
    Younger(u32),
}

impl Filter {
    pub fn and(filters: impl IntoIterator<Item = Filter>) -> Filter {
        Filter::Operator(Operator::And, filters.into_iter().collect())
    }
    pub fn or(filters: impl IntoIterator<Item = Filter>) -> Filter {
        Filter::Operator(Operator::Or, filters.into_iter().collect())
    }
    pub fn not(filters: impl IntoIterator<Item = Filter>) -> Filter {
        Filter::Operator(Operator::Not, filters.into_iter().collect())
    }

    pub fn seq_saved_search() -> Filter {
        Filter::Sequence(Sequence::SavedSearch, false)
    }

    pub fn seq_range(start: Option<u32>, end: Option<u32>) -> Filter {
        Filter::Sequence(Sequence::Range { start, end }, false)
    }
}

impl ImapResponse for Response {
    fn serialize(&self, tag: String, version: ProtocolVersion) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);
        if version.is_rev2() {
            buf.extend_from_slice(b"* ESEARCH (TAG ");
            quoted_string(&mut buf, &tag);
            buf.extend_from_slice(b")");
            if let Some(count) = &self.count {
                buf.extend_from_slice(b" COUNT ");
                buf.extend_from_slice(count.to_string().as_bytes());
            }
            if let Some(min) = &self.min {
                buf.extend_from_slice(b" MIN ");
                buf.extend_from_slice(min.to_string().as_bytes());
            }
            if let Some(max) = &self.max {
                buf.extend_from_slice(b" MAX ");
                buf.extend_from_slice(max.to_string().as_bytes());
            }
            if !self.ids.is_empty() {
                buf.extend_from_slice(b" ALL ");
                serialize_sequence(&mut buf, &self.ids);
            }
        } else {
            buf.extend_from_slice(b"* SEARCH");
            if !self.ids.is_empty() {
                for id in &self.ids {
                    buf.push(b' ');
                    buf.extend_from_slice(id.to_string().as_bytes());
                }
            }
        }
        buf.extend_from_slice(b"\r\n");
        StatusResponse::completed(Command::Search(self.is_uid), tag).serialize(&mut buf);
        buf
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::{ImapResponse, ProtocolVersion};

    #[test]
    fn serialize_search() {
        for (response, tag, expected_v2, expected_v1) in [
            (
                super::Response {
                    is_uid: false,
                    ids: vec![2, 10, 11],
                    min: 2.into(),
                    max: 11.into(),
                    count: 3.into(),
                },
                "A283",
                concat!(
                    "* ESEARCH (TAG \"A283\") COUNT 3 MIN 2 MAX 11 ALL 2,10:11\r\n",
                    "A283 OK SEARCH completed\r\n"
                ),
                concat!("* SEARCH 2 10 11\r\n", "A283 OK SEARCH completed\r\n"),
            ),
            (
                super::Response {
                    is_uid: false,
                    ids: vec![
                        1, 2, 3, 5, 10, 11, 12, 13, 90, 92, 93, 94, 95, 96, 97, 98, 99,
                    ],
                    min: None,
                    max: None,
                    count: None,
                },
                "A283",
                concat!(
                    "* ESEARCH (TAG \"A283\") ALL 1:3,5,10:13,90,92:99\r\n",
                    "A283 OK SEARCH completed\r\n"
                ),
                concat!(
                    "* SEARCH 1 2 3 5 10 11 12 13 90 92 93 94 95 96 97 98 99\r\n",
                    "A283 OK SEARCH completed\r\n"
                ),
            ),
            (
                super::Response {
                    is_uid: false,
                    ids: vec![],
                    min: None,
                    max: None,
                    count: None,
                },
                "A283",
                concat!(
                    "* ESEARCH (TAG \"A283\")\r\n",
                    "A283 OK SEARCH completed\r\n"
                ),
                concat!("* SEARCH\r\n", "A283 OK SEARCH completed\r\n"),
            ),
        ] {
            let response_v1 =
                String::from_utf8(response.serialize(tag.to_string(), ProtocolVersion::Rev1))
                    .unwrap();
            let response_v2 =
                String::from_utf8(response.serialize(tag.to_string(), ProtocolVersion::Rev2))
                    .unwrap();

            assert_eq!(response_v2, expected_v2);
            assert_eq!(response_v1, expected_v1);
        }
    }
}
