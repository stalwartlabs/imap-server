use super::{serialize_sequence, ImapResponse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub is_uid: bool,
    pub is_qresync: bool,
    pub ids: Vec<u32>,
}

impl ImapResponse for Response {
    fn serialize(self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);
        if !self.is_qresync {
            for id in &self.ids {
                buf.extend_from_slice(b"* ");
                buf.extend_from_slice(id.to_string().as_bytes());
                buf.extend_from_slice(b" EXPUNGE\r\n");
            }
        } else {
            Vanished {
                earlier: false,
                ids: self.ids,
            }
            .serialize(&mut buf);
        }
        buf
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vanished {
    pub earlier: bool,
    pub ids: Vec<u32>,
}

impl Vanished {
    pub fn serialize(&self, buf: &mut Vec<u8>) {
        if self.earlier {
            buf.extend_from_slice(b"* VANISHED (EARLIER) ");
        } else {
            buf.extend_from_slice(b"* VANISHED ");
        }
        serialize_sequence(buf, &self.ids);
        buf.extend_from_slice(b"\r\n");
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::ImapResponse;

    #[test]
    fn serialize_expunge() {
        assert_eq!(
            String::from_utf8(
                super::Response {
                    is_qresync: false,
                    is_uid: false,
                    ids: vec![3, 4, 5]
                }
                .serialize()
            )
            .unwrap(),
            concat!("* 3 EXPUNGE\r\n", "* 4 EXPUNGE\r\n", "* 5 EXPUNGE\r\n",)
        );

        assert_eq!(
            String::from_utf8(
                super::Response {
                    is_qresync: true,
                    is_uid: false,
                    ids: vec![3, 4, 5]
                }
                .serialize()
            )
            .unwrap(),
            concat!("* VANISHED 3:5\r\n")
        );
    }
}
