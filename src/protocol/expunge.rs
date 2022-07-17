use crate::core::{Command, StatusResponse};

use super::ImapResponse;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub is_uid: bool,
    pub ids: Vec<u32>,
}

impl ImapResponse for Response {
    fn serialize(&self, tag: String) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);
        for id in &self.ids {
            buf.extend_from_slice(b"* ");
            buf.extend_from_slice(id.to_string().as_bytes());
            buf.extend_from_slice(b" EXPUNGE\r\n");
        }
        StatusResponse::completed(Command::Expunge(self.is_uid), tag).serialize(&mut buf);
        buf
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::ImapResponse;

    #[test]
    fn serialize_expunge() {
        assert_eq!(
            &super::Response {
                is_uid: false,
                ids: vec![3, 5, 8]
            }
            .serialize("A202".to_string()),
            concat!(
                "* 3 EXPUNGE\r\n",
                "* 5 EXPUNGE\r\n",
                "* 8 EXPUNGE\r\n",
                "A202 OK EXPUNGE completed\r\n"
            )
            .as_bytes()
        );
    }
}
