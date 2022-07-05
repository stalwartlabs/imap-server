use crate::{core::receiver::Request, protocol::create};

pub fn parse_create(request: Request) -> crate::core::Result<create::Arguments> {
    match request.tokens.len() {
        1 => Ok(create::Arguments {
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
    use crate::{core::receiver::Receiver, protocol::create};

    #[test]
    fn parse_create() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                "A142 CREATE 12345\r\n",
                create::Arguments {
                    name: "12345".to_string(),
                },
            ),
            (
                "A142 CREATE \"my funky mailbox\"\r\n",
                create::Arguments {
                    name: "my funky mailbox".to_string(),
                },
            ),
        ] {
            assert_eq!(
                super::parse_create(receiver.parse(&mut command.as_bytes().iter()).unwrap())
                    .unwrap(),
                arguments
            );
        }
    }
}
