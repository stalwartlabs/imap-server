use crate::{core::receiver::Request, protocol::subscribe};

pub fn parse_subscribe(request: Request) -> crate::core::Result<subscribe::Arguments> {
    match request.tokens.len() {
        1 => Ok(subscribe::Arguments {
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
    use crate::{core::receiver::Receiver, protocol::subscribe};

    #[test]
    fn parse_subscribe() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                "A142 SUBSCRIBE #news.comp.mail.mime\r\n",
                subscribe::Arguments {
                    name: "#news.comp.mail.mime".to_string(),
                },
            ),
            (
                "A142 SUBSCRIBE \"#news.comp.mail.mime\"\r\n",
                subscribe::Arguments {
                    name: "#news.comp.mail.mime".to_string(),
                },
            ),
        ] {
            assert_eq!(
                super::parse_subscribe(receiver.parse(&mut command.as_bytes().iter()).unwrap())
                    .unwrap(),
                arguments
            );
        }
    }
}
