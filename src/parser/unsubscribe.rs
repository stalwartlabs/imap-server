use crate::{core::receiver::Request, protocol::unsubscribe};

pub fn parse_unsubscribe(request: Request) -> crate::core::Result<unsubscribe::Arguments> {
    match request.tokens.len() {
        1 => Ok(unsubscribe::Arguments {
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
    use crate::{core::receiver::Receiver, protocol::unsubscribe};

    #[test]
    fn parse_unsubscribe() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                "A142 UNSUBSCRIBE #news.comp.mail.mime\r\n",
                unsubscribe::Arguments {
                    name: "#news.comp.mail.mime".to_string(),
                },
            ),
            (
                "A142 UNSUBSCRIBE \"#news.comp.mail.mime\"\r\n",
                unsubscribe::Arguments {
                    name: "#news.comp.mail.mime".to_string(),
                },
            ),
        ] {
            assert_eq!(
                super::parse_unsubscribe(receiver.parse(&mut command.as_bytes().iter()).unwrap())
                    .unwrap(),
                arguments
            );
        }
    }
}
