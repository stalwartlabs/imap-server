use crate::{core::receiver::Token, protocol::create};

pub fn parse_create(tokens: Vec<Token>) -> super::Result<create::Arguments> {
    match tokens.len() {
        1 => Ok(create::Arguments {
            name: tokens.into_iter().next().unwrap().unwrap_string()?,
        }),
        0 => Err("Missing mailbox name.".into()),
        _ => Err("Too many arguments.".into()),
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
                super::parse_create(
                    receiver
                        .parse(&mut command.as_bytes().iter())
                        .unwrap()
                        .tokens
                )
                .unwrap(),
                arguments
            );
        }
    }
}
