use crate::{
    core::{receiver::Request, utf7::utf7_maybe_decode},
    protocol::{unsubscribe, ProtocolVersion},
};

impl Request {
    pub fn parse_unsubscribe(
        self,
        version: ProtocolVersion,
    ) -> crate::core::Result<unsubscribe::Arguments> {
        match self.tokens.len() {
            1 => Ok(unsubscribe::Arguments {
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
        protocol::{unsubscribe, ProtocolVersion},
    };

    #[test]
    fn parse_unsubscribe() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                "A142 UNSUBSCRIBE #news.comp.mail.mime\r\n",
                unsubscribe::Arguments {
                    mailbox_name: "#news.comp.mail.mime".to_string(),
                    tag: "A142".to_string(),
                },
            ),
            (
                "A142 UNSUBSCRIBE \"#news.comp.mail.mime\"\r\n",
                unsubscribe::Arguments {
                    mailbox_name: "#news.comp.mail.mime".to_string(),
                    tag: "A142".to_string(),
                },
            ),
        ] {
            assert_eq!(
                receiver
                    .parse(&mut command.as_bytes().iter())
                    .unwrap()
                    .parse_unsubscribe(ProtocolVersion::Rev2)
                    .unwrap(),
                arguments
            );
        }
    }
}
