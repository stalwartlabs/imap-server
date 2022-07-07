use crate::{
    core::{receiver::Request, utf7::utf7_maybe_decode},
    protocol::{examine, ProtocolVersion},
};

impl Request {
    pub fn parse_examine(
        self,
        version: ProtocolVersion,
    ) -> crate::core::Result<examine::Arguments> {
        match self.tokens.len() {
            1 => Ok(examine::Arguments {
                mailbox_name: utf7_maybe_decode(
                    self.tokens
                        .into_iter()
                        .next()
                        .unwrap()
                        .unwrap_string()
                        .map_err(|v| (self.tag.as_ref(), v))?,
                    version,
                ),
                tag: self.tag,
            }),
            0 => Err(self.into_error("Missing mailbox name.")),
            _ => Err(self.into_error("Too many arguments.")),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        core::receiver::Receiver,
        protocol::{examine, ProtocolVersion},
    };

    #[test]
    fn parse_examine() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                "A142 EXAMINE INBOX\r\n",
                examine::Arguments {
                    mailbox_name: "INBOX".to_string(),
                    tag: "A142".to_string(),
                },
            ),
            (
                "A142 EXAMINE {4+}\r\ntest\r\n",
                examine::Arguments {
                    mailbox_name: "test".to_string(),
                    tag: "A142".to_string(),
                },
            ),
        ] {
            assert_eq!(
                receiver
                    .parse(&mut command.as_bytes().iter())
                    .unwrap()
                    .parse_examine(ProtocolVersion::Rev2)
                    .unwrap(),
                arguments
            );
        }
    }
}
