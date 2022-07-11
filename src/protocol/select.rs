use crate::core::{Command, ResponseCode, StatusResponse};

use super::{
    list::ListItem,
    ImapResponse,
    ProtocolVersion::{self},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arguments {
    pub tag: String,
    pub mailbox_name: String,
}

pub struct Response {
    pub mailbox: ListItem,
    pub total_messages: usize,
    pub recent_messages: usize,
    pub unseen_seq: u32,
    pub uid_validity: u32,
    pub uid_next: u32,
    pub is_read_only: bool,
    pub is_examine: bool,
    pub closed_previous: bool,
}

impl ImapResponse for Response {
    fn serialize(&self, tag: String, version: ProtocolVersion) -> Vec<u8> {
        let mut buf = Vec::with_capacity(100);
        if self.closed_previous {
            StatusResponse::ok(None, ResponseCode::Closed.into(), "Closed previous mailbox")
                .serialize(&mut buf);
        }
        buf.extend_from_slice(b"* ");
        buf.extend_from_slice(self.total_messages.to_string().as_bytes());
        buf.extend_from_slice(
            b" EXISTS\r\n* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n",
        );
        if version.is_rev2() {
            self.mailbox.serialize(&mut buf, version, false);
        } else {
            buf.extend_from_slice(b"* ");
            buf.extend_from_slice(self.recent_messages.to_string().as_bytes());
            buf.extend_from_slice(b" RECENT\r\n");
            if self.unseen_seq > 0 {
                buf.extend_from_slice(b"* OK [UNSEEN ");
                buf.extend_from_slice(self.unseen_seq.to_string().as_bytes());
                buf.extend_from_slice(b"]\r\n");
            }
        }
        buf.extend_from_slice(
            b"* OK [PERMANENTFLAGS (\\Deleted \\Seen \\Answered \\Flagged \\Draft \\*)]\r\n",
        );
        buf.extend_from_slice(b"* OK [UIDVALIDITY ");
        buf.extend_from_slice(self.uid_validity.to_string().as_bytes());
        buf.extend_from_slice(b"]\r\n* OK [UIDNEXT ");
        buf.extend_from_slice(self.uid_next.to_string().as_bytes());
        buf.extend_from_slice(b"]\r\n");

        StatusResponse::completed(
            if !self.is_examine {
                Command::Select
            } else {
                Command::Examine
            },
            tag,
        )
        .with_code(if !self.is_read_only {
            ResponseCode::ReadWrite
        } else {
            ResponseCode::ReadOnly
        })
        .serialize(&mut buf);
        buf
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::{list::ListItem, ImapResponse, ProtocolVersion};

    #[test]
    fn serialize_select() {
        for (response, tag, expected_v2, expected_v1) in [
            (
                super::Response {
                    mailbox: ListItem::new("INBOX"),
                    total_messages: 172,
                    recent_messages: 5,
                    unseen_seq: 3,
                    uid_validity: 3857529045,
                    uid_next: 4392,
                    is_read_only: false,
                    is_examine: false,
                    closed_previous: false,
                },
                "A142",
                concat!(
                    "* 172 EXISTS\r\n",
                    "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n",
                    "* LIST () \"/\" \"INBOX\"\r\n",
                    "* OK [PERMANENTFLAGS (\\Deleted \\Seen \\Answered \\Flagged \\Draft \\*)]\r\n",
                    "* OK [UIDVALIDITY 3857529045]\r\n",
                    "* OK [UIDNEXT 4392]\r\n",
                    "A142 OK [READ-WRITE] SELECT completed\r\n"
                ),
                concat!(
                    "* 172 EXISTS\r\n",
                    "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n",
                    "* 5 RECENT\r\n",
                    "* OK [UNSEEN 3]\r\n",
                    "* OK [PERMANENTFLAGS (\\Deleted \\Seen \\Answered \\Flagged \\Draft \\*)]\r\n",
                    "* OK [UIDVALIDITY 3857529045]\r\n",
                    "* OK [UIDNEXT 4392]\r\n",
                    "A142 OK [READ-WRITE] SELECT completed\r\n"
                ),
            ),
            (
                super::Response {
                    mailbox: ListItem::new("~peter/mail/台北/日本語"),
                    total_messages: 172,
                    recent_messages: 5,
                    unseen_seq: 3,
                    uid_validity: 3857529045,
                    uid_next: 4392,
                    is_read_only: true,
                    is_examine: false,
                    closed_previous: true,
                },
                "A142",
                concat!(
                    "* OK [CLOSED] Closed previous mailbox\r\n",
                    "* 172 EXISTS\r\n",
                    "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n",
                    "* LIST () \"/\" \"~peter/mail/台北/日本語\" (\"OLDNAME\" (\"~peter/mail/&U,BTFw-/&ZeVnLIqe-\"))\r\n",
                    "* OK [PERMANENTFLAGS (\\Deleted \\Seen \\Answered \\Flagged \\Draft \\*)]\r\n",
                    "* OK [UIDVALIDITY 3857529045]\r\n",
                    "* OK [UIDNEXT 4392]\r\n",
                    "A142 OK [READ-ONLY] SELECT completed\r\n"
                ),
                concat!(
                    "* OK [CLOSED] Closed previous mailbox\r\n",
                    "* 172 EXISTS\r\n",
                    "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n",
                    "* 5 RECENT\r\n",
                    "* OK [UNSEEN 3]\r\n",
                    "* OK [PERMANENTFLAGS (\\Deleted \\Seen \\Answered \\Flagged \\Draft \\*)]\r\n",
                    "* OK [UIDVALIDITY 3857529045]\r\n",
                    "* OK [UIDNEXT 4392]\r\n",
                    "A142 OK [READ-ONLY] SELECT completed\r\n"
                ),
            ),
        ] {
            let response_v1 = String::from_utf8(response.serialize(tag.to_string(), ProtocolVersion::Rev1)).unwrap();
            let response_v2 = String::from_utf8(response.serialize(tag.to_string(), ProtocolVersion::Rev2)).unwrap();

            assert_eq!(response_v2, expected_v2);
            assert_eq!(response_v1, expected_v1);
        }
    }
}
