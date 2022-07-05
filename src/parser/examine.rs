use crate::{core::receiver::Request, protocol::examine};

pub fn parse_examine(request: Request) -> crate::core::Result<examine::Arguments> {
    match request.tokens.len() {
        1 => Ok(examine::Arguments {
            name: request
                .tokens
                .into_iter()
                .next()
                .unwrap()
                .unwrap_string()
                .map_err(|v| (request.tag, v))?,
        }),
        0 => Err(request.into_error("Missing mailbox name.")),
        _ => Err(request.into_error("Too many arguments.")),
    }
}

#[cfg(test)]
mod tests {
    use crate::{core::receiver::Receiver, protocol::examine};

    #[test]
    fn parse_examine() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                "A142 EXAMINE INBOX\r\n",
                examine::Arguments {
                    name: "INBOX".to_string(),
                },
            ),
            (
                "A142 EXAMINE {4+}\r\ntest\r\n",
                examine::Arguments {
                    name: "test".to_string(),
                },
            ),
        ] {
            assert_eq!(
                super::parse_examine(receiver.parse(&mut command.as_bytes().iter()).unwrap())
                    .unwrap(),
                arguments
            );
        }
    }
}
