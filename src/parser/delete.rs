use crate::{core::receiver::Request, protocol::delete};

pub fn parse_delete(request: Request) -> crate::core::Result<delete::Arguments> {
    match request.tokens.len() {
        1 => Ok(delete::Arguments {
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
    use crate::{core::receiver::Receiver, protocol::delete};

    #[test]
    fn parse_delete() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                "A142 DELETE INBOX\r\n",
                delete::Arguments {
                    name: "INBOX".to_string(),
                },
            ),
            (
                "A142 DELETE \"my funky mailbox\"\r\n",
                delete::Arguments {
                    name: "my funky mailbox".to_string(),
                },
            ),
        ] {
            assert_eq!(
                super::parse_delete(receiver.parse(&mut command.as_bytes().iter()).unwrap())
                    .unwrap(),
                arguments
            );
        }
    }
}
