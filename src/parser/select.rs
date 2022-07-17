use crate::{
    core::{
        receiver::{Request, Token},
        utf7::utf7_maybe_decode,
        StatusResponse,
    },
    protocol::{select, ProtocolVersion},
};

impl Request {
    pub fn parse_select(self, version: ProtocolVersion) -> crate::core::Result<select::Arguments> {
        if !self.tokens.is_empty() {
            let mut tokens = self.tokens.into_iter();

            // Mailbox name
            let mailbox_name = utf7_maybe_decode(
                tokens
                    .next()
                    .unwrap()
                    .unwrap_string()
                    .map_err(|v| (self.tag.as_ref(), v))?,
                version,
            );

            // CONDSTORE parameters
            let mut condstore = false;
            match tokens.next() {
                Some(Token::ParenthesisOpen) => {
                    for token in tokens {
                        match token {
                            Token::Argument(param) if param.eq_ignore_ascii_case(b"CONDSTORE") => {
                                condstore = true;
                            }
                            Token::ParenthesisClose => {
                                break;
                            }
                            _ => {
                                return Err(StatusResponse::bad(
                                    self.tag.into(),
                                    None,
                                    format!("Unexpected value '{}'.", token),
                                ));
                            }
                        }
                    }
                }
                Some(token) => {
                    return Err(StatusResponse::bad(
                        self.tag.into(),
                        None,
                        format!("Unexpected value '{}'.", token),
                    ));
                }
                None => (),
            }

            Ok(select::Arguments {
                mailbox_name,
                tag: self.tag,
                condstore,
            })
        } else {
            Err(self.into_error("Missing mailbox name."))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        core::receiver::Receiver,
        protocol::{select, ProtocolVersion},
    };

    #[test]
    fn parse_select() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                "A142 SELECT INBOX\r\n",
                select::Arguments {
                    mailbox_name: "INBOX".to_string(),
                    tag: "A142".to_string(),
                    condstore: false,
                },
            ),
            (
                "A142 SELECT \"my funky mailbox\"\r\n",
                select::Arguments {
                    mailbox_name: "my funky mailbox".to_string(),
                    tag: "A142".to_string(),
                    condstore: false,
                },
            ),
            (
                "A142 SELECT INBOX (CONDSTORE)\r\n",
                select::Arguments {
                    mailbox_name: "INBOX".to_string(),
                    tag: "A142".to_string(),
                    condstore: true,
                },
            ),
        ] {
            assert_eq!(
                receiver
                    .parse(&mut command.as_bytes().iter())
                    .unwrap()
                    .parse_select(ProtocolVersion::Rev2)
                    .unwrap(),
                arguments
            );
        }
    }
}
