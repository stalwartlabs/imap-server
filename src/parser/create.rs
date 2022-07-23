use jmap_client::mailbox::Role;

use crate::{
    core::{
        receiver::{Request, Token},
        utf7::utf7_maybe_decode,
    },
    protocol::{create, ProtocolVersion},
};

impl Request {
    pub fn parse_create(self, version: ProtocolVersion) -> crate::core::Result<create::Arguments> {
        if !self.tokens.is_empty() {
            let mut tokens = self.tokens.into_iter();
            let mailbox_name = utf7_maybe_decode(
                tokens
                    .next()
                    .unwrap()
                    .unwrap_string()
                    .map_err(|v| (self.tag.as_ref(), v))?,
                version,
            );
            let mailbox_role = if let Some(Token::ParenthesisOpen) = tokens.next() {
                match tokens.next() {
                    Some(Token::Argument(param)) if param.eq_ignore_ascii_case(b"USE") => (),
                    _ => {
                        return Err((self.tag, "Failed to parse, expected 'USE'.").into());
                    }
                }
                if tokens
                    .next()
                    .map_or(true, |token| !token.is_parenthesis_open())
                {
                    return Err((self.tag, "Expected '(' after 'USE'.").into());
                }
                match tokens.next() {
                    Some(Token::Argument(value)) => {
                        if value.eq_ignore_ascii_case(b"\\Archive") {
                            Role::Archive
                        } else if value.eq_ignore_ascii_case(b"\\Drafts") {
                            Role::Drafts
                        } else if value.eq_ignore_ascii_case(b"\\Junk") {
                            Role::Junk
                        } else if value.eq_ignore_ascii_case(b"\\Sent") {
                            Role::Sent
                        } else if value.eq_ignore_ascii_case(b"\\Trash") {
                            Role::Trash
                        } else if value.eq_ignore_ascii_case(b"\\Important") {
                            Role::Important
                        } else if value.eq_ignore_ascii_case(b"\\All") {
                            return Err((
                                self.tag,
                                "A mailbox with the \"\\All\" attribute already exists.",
                            )
                                .into());
                        } else {
                            return Err((
                                self.tag,
                                format!(
                                    "Special use attribute {:?} is not supported.",
                                    String::from_utf8_lossy(&value)
                                ),
                            )
                                .into());
                        }
                    }
                    _ => {
                        return Err((self.tag, "Invalid SPECIAL-USE attribute.").into());
                    }
                }
            } else {
                Role::None
            };

            Ok(create::Arguments {
                mailbox_name,
                mailbox_role,
                tag: self.tag,
            })
        } else {
            Err(self.into_error("Too many arguments."))
        }
    }
}

#[cfg(test)]
mod tests {
    use jmap_client::mailbox::Role;

    use crate::{
        core::receiver::Receiver,
        protocol::{create, ProtocolVersion},
    };

    #[test]
    fn parse_create() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                "A142 CREATE 12345\r\n",
                create::Arguments {
                    tag: "A142".to_string(),
                    mailbox_name: "12345".to_string(),
                    mailbox_role: Role::None,
                },
            ),
            (
                "A142 CREATE \"my funky mailbox\"\r\n",
                create::Arguments {
                    tag: "A142".to_string(),
                    mailbox_name: "my funky mailbox".to_string(),
                    mailbox_role: Role::None,
                },
            ),
            (
                "t1 CREATE \"Important Messages\" (USE (\\Important))\r\n",
                create::Arguments {
                    tag: "t1".to_string(),
                    mailbox_name: "Important Messages".to_string(),
                    mailbox_role: Role::Important,
                },
            ),
        ] {
            assert_eq!(
                receiver
                    .parse(&mut command.as_bytes().iter())
                    .unwrap()
                    .parse_create(ProtocolVersion::Rev2)
                    .unwrap(),
                arguments
            );
        }
    }
}
