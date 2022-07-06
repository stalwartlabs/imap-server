use crate::{
    core::{receiver::Request, utf7::utf7_maybe_decode},
    protocol::{create, ProtocolVersion},
};

impl Request {
    pub fn parse_create(self, version: ProtocolVersion) -> crate::core::Result<create::Arguments> {
        match self.tokens.len() {
            1 => Ok(create::Arguments {
                name: utf7_maybe_decode(
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
                    name: "12345".to_string(),
                },
            ),
            (
                "A142 CREATE \"my funky mailbox\"\r\n",
                create::Arguments {
                    tag: "A142".to_string(),
                    name: "my funky mailbox".to_string(),
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
