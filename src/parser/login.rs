use crate::{core::receiver::Request, protocol::login};

pub fn parse_login(request: Request) -> crate::core::Result<login::Arguments> {
    match request.tokens.len() {
        2 => {
            let mut tokens = request.tokens.into_iter();
            Ok(login::Arguments {
                username: tokens
                    .next()
                    .unwrap()
                    .unwrap_string()
                    .map_err(|v| (request.tag.as_str(), v))?,
                password: tokens
                    .next()
                    .unwrap()
                    .unwrap_string()
                    .map_err(|v| (request.tag.as_str(), v))?,
                tag: request.tag,
            })
        }
        0 => Err(request.into_error("Missing arguments.")),
        _ => Err(request.into_error("Too many arguments.")),
    }
}

#[cfg(test)]
mod tests {
    use crate::{core::receiver::Receiver, protocol::login};

    #[test]
    fn parse_login() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                "a001 LOGIN SMITH SESAME\r\n",
                login::Arguments {
                    tag: "a001".to_string(),
                    username: "SMITH".to_string(),
                    password: "SESAME".to_string(),
                },
            ),
            (
                "A001 LOGIN {11+}\r\nFRED FOOBAR {7+}\r\nfat man\r\n",
                login::Arguments {
                    tag: "A001".to_string(),
                    username: "FRED FOOBAR".to_string(),
                    password: "fat man".to_string(),
                },
            ),
        ] {
            assert_eq!(
                super::parse_login(receiver.parse(&mut command.as_bytes().iter()).unwrap())
                    .unwrap(),
                arguments
            );
        }
    }
}
