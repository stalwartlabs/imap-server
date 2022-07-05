use crate::{core::receiver::Request, protocol::select};

pub fn parse_select(request: Request) -> crate::core::Result<select::Arguments> {
    match request.tokens.len() {
        1 => Ok(select::Arguments {
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
    use crate::{core::receiver::Receiver, protocol::select};

    #[test]
    fn parse_select() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                "A142 SELECT INBOX\r\n",
                select::Arguments {
                    name: "INBOX".to_string(),
                },
            ),
            (
                "A142 SELECT \"my funky mailbox\"\r\n",
                select::Arguments {
                    name: "my funky mailbox".to_string(),
                },
            ),
        ] {
            assert_eq!(
                super::parse_select(receiver.parse(&mut command.as_bytes().iter()).unwrap())
                    .unwrap(),
                arguments
            );
        }
    }
}
