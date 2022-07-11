use crate::core::{Command, StatusResponse};

use super::{authenticate::Mechanism, ImapResponse, ProtocolVersion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub capabilities: Vec<Capability>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    IMAP4rev2,
    IMAP4rev1,
    StartTLS,
    LoginDisabled,
    Condstore,
    Auth(Mechanism),
}

impl Capability {
    pub fn serialize(&self, buf: &mut Vec<u8>) {
        match self {
            Capability::IMAP4rev2 => {
                buf.extend_from_slice(b"IMAP4rev2");
            }
            Capability::IMAP4rev1 => {
                buf.extend_from_slice(b"IMAP4rev1");
            }
            Capability::StartTLS => {
                buf.extend_from_slice(b"STARTTLS");
            }
            Capability::LoginDisabled => {
                buf.extend_from_slice(b"LOGINDISABLED");
            }
            Capability::Condstore => {
                buf.extend_from_slice(b"CONDSTORE");
            }
            Capability::Auth(mechanism) => {
                buf.extend_from_slice(b"AUTH=");
                mechanism.serialize(buf);
            }
        }
    }
}

impl ImapResponse for Response {
    fn serialize(&self, tag: String, _imap_rev: ProtocolVersion) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(b"* CAPABILITY");
        for capability in self.capabilities.iter() {
            buf.push(b' ');
            capability.serialize(&mut buf);
        }
        buf.extend_from_slice(b"\r\n");
        StatusResponse::completed(Command::Capability, tag).serialize(&mut buf);
        buf
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::{
        capability::{Capability, Response},
        ImapResponse, ProtocolVersion,
    };

    #[test]
    fn serialize_capability() {
        assert_eq!(
            &Response {
                capabilities: vec![
                    Capability::IMAP4rev2,
                    Capability::StartTLS,
                    Capability::LoginDisabled
                ],
            }
            .serialize("a003".to_string(), ProtocolVersion::Rev2),
            concat!(
                "* CAPABILITY IMAP4rev2 STARTTLS LOGINDISABLED\r\n",
                "a003 OK CAPABILITY completed\r\n"
            )
            .as_bytes()
        );
    }
}
