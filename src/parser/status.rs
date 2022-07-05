use crate::core::receiver::{Request, Token};
use crate::protocol::status;
use crate::protocol::status::Status;

pub fn parse_status(request: Request) -> crate::core::Result<status::Arguments> {
    match request.tokens.len() {
        0..=3 => Err(request.into_error("Missing arguments.")),
        len => {
            let mut tokens = request.tokens.into_iter();
            let name = tokens
                .next()
                .unwrap()
                .unwrap_string()
                .map_err(|v| (request.tag.as_str(), v))?;
            let mut items = Vec::with_capacity(len - 2);

            if tokens
                .next()
                .map_or(true, |token| !token.is_parenthesis_open())
            {
                return Err((
                    request.tag.as_str(),
                    "Expected parenthesis after mailbox name.",
                )
                    .into());
            }

            #[allow(clippy::while_let_on_iterator)]
            while let Some(token) = tokens.next() {
                match token {
                    Token::ParenthesisClose => break,
                    Token::Argument(value) => {
                        items.push(Status::parse(&value).map_err(|v| (request.tag.as_str(), v))?);
                    }
                    _ => {
                        return Err((
                            request.tag.as_str(),
                            "Invalid status return option argument.",
                        )
                            .into())
                    }
                }
            }

            Ok(status::Arguments { name, items })
        }
    }
}

impl Status {
    pub fn parse(value: &[u8]) -> super::Result<Self> {
        if value.eq_ignore_ascii_case(b"messages") {
            Ok(Self::Messages)
        } else if value.eq_ignore_ascii_case(b"uidnext") {
            Ok(Self::UidNext)
        } else if value.eq_ignore_ascii_case(b"uidvalidity") {
            Ok(Self::UidValidity)
        } else if value.eq_ignore_ascii_case(b"unseen") {
            Ok(Self::Unseen)
        } else if value.eq_ignore_ascii_case(b"deleted") {
            Ok(Self::Deleted)
        } else if value.eq_ignore_ascii_case(b"size") {
            Ok(Self::Size)
        } else {
            Err(format!(
                "Invalid status option '{}'.",
                String::from_utf8_lossy(value)
            )
            .into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{core::receiver::Receiver, protocol::status};

    #[test]
    fn parse_status() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [(
            "A042 STATUS blurdybloop (UIDNEXT MESSAGES)\r\n",
            status::Arguments {
                name: "blurdybloop".to_string(),
                items: vec![status::Status::UidNext, status::Status::Messages],
            },
        )] {
            assert_eq!(
                super::parse_status(receiver.parse(&mut command.as_bytes().iter()).unwrap())
                    .unwrap(),
                arguments
            );
        }
    }
}
