use crate::core::receiver::{Request, Token};
use crate::core::utf7::utf7_maybe_decode;
use crate::protocol::status::Status;
use crate::protocol::{status, ProtocolVersion};

impl Request {
    pub fn parse_status(self, version: ProtocolVersion) -> crate::core::Result<status::Arguments> {
        match self.tokens.len() {
            0..=3 => Err(self.into_error("Missing arguments.")),
            len => {
                let mut tokens = self.tokens.into_iter();
                let mailbox_name = utf7_maybe_decode(
                    tokens
                        .next()
                        .unwrap()
                        .unwrap_string()
                        .map_err(|v| (self.tag.as_ref(), v))?,
                    version,
                );
                let mut items = Vec::with_capacity(len - 2);

                if tokens
                    .next()
                    .map_or(true, |token| !token.is_parenthesis_open())
                {
                    return Err((
                        self.tag.as_str(),
                        "Expected parenthesis after mailbox name.",
                    )
                        .into());
                }

                #[allow(clippy::while_let_on_iterator)]
                while let Some(token) = tokens.next() {
                    match token {
                        Token::ParenthesisClose => break,
                        Token::Argument(value) => {
                            items.push(Status::parse(&value).map_err(|v| (self.tag.as_str(), v))?);
                        }
                        _ => {
                            return Err((
                                self.tag.as_str(),
                                "Invalid status return option argument.",
                            )
                                .into())
                        }
                    }
                }

                if !items.is_empty() {
                    Ok(status::Arguments {
                        tag: self.tag,
                        mailbox_name,
                        items,
                    })
                } else {
                    Err((self.tag, "At least one status item is required.").into())
                }
            }
        }
    }
}

impl Status {
    pub fn parse(value: &[u8]) -> super::Result<Self> {
        if value.eq_ignore_ascii_case(b"messages") {
            Ok(Self::Messages)
        } else if value.eq_ignore_ascii_case(b"uidnext") {
            Ok(Self::UidNext)
        } else if value.eq_ignore_ascii_case(b"uidvalidity") {
            Ok(Self::UidValidity)
        } else if value.eq_ignore_ascii_case(b"unseen") {
            Ok(Self::Unseen)
        } else if value.eq_ignore_ascii_case(b"deleted") {
            Ok(Self::Deleted)
        } else if value.eq_ignore_ascii_case(b"size") {
            Ok(Self::Size)
        } else {
            Err(format!(
                "Invalid status option '{}'.",
                String::from_utf8_lossy(value)
            )
            .into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        core::receiver::Receiver,
        protocol::{status, ProtocolVersion},
    };

    #[test]
    fn parse_status() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [(
            "A042 STATUS blurdybloop (UIDNEXT MESSAGES)\r\n",
            status::Arguments {
                tag: "A042".to_string(),
                mailbox_name: "blurdybloop".to_string(),
                items: vec![status::Status::UidNext, status::Status::Messages],
            },
        )] {
            assert_eq!(
                receiver
                    .parse(&mut command.as_bytes().iter())
                    .unwrap()
                    .parse_status(ProtocolVersion::Rev2)
                    .unwrap(),
                arguments
            );
        }
    }
}
