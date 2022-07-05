use crate::{core::receiver::Request, protocol::rename};

pub fn parse_rename(request: Request) -> crate::core::Result<rename::Arguments> {
    match request.tokens.len() {
        2 => {
            let mut tokens = request.tokens.into_iter();
            Ok(rename::Arguments {
                name: tokens
                    .next()
                    .unwrap()
                    .unwrap_string()
                    .map_err(|v| (request.tag.as_str(), v))?,
                new_name: tokens
                    .next()
                    .unwrap()
                    .unwrap_string()
                    .map_err(|v| (request.tag.as_str(), v))?,
            })
        }
        0 => Err(request.into_error("Missing argument.")),
        1 => Err(request.into_error("Missing new mailbox name.")),
        _ => Err(request.into_error("Too many arguments.")),
    }
}

#[cfg(test)]
mod tests {
    use crate::{core::receiver::Receiver, protocol::rename};

    #[test]
    fn parse_rename() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                "A142 RENAME \"my funky mailbox\" Private\r\n",
                rename::Arguments {
                    name: "my funky mailbox".to_string(),
                    new_name: "Private".to_string(),
                },
            ),
            (
                "A142 RENAME {1+}\r\na {1+}\r\nb\r\n",
                rename::Arguments {
                    name: "a".to_string(),
                    new_name: "b".to_string(),
                },
            ),
        ] {
            assert_eq!(
                super::parse_rename(receiver.parse(&mut command.as_bytes().iter()).unwrap())
                    .unwrap(),
                arguments
            );
        }
    }
}
