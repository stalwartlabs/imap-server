use crate::{core::receiver::Token, protocol::login};

pub fn parse_login(tokens: Vec<Token>) -> super::Result<login::Arguments> {
    match tokens.len() {
        2 => {
            let mut tokens = tokens.into_iter();
            Ok(login::Arguments {
                username: tokens.next().unwrap().unwrap_string()?,
                password: tokens.next().unwrap().unwrap_string()?,
            })
        }
        0 => Err("Missing arguments.".into()),
        _ => Err("Too many arguments.".into()),
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
                    username: "SMITH".to_string(),
                    password: "SESAME".to_string(),
                },
            ),
            (
                "A001 LOGIN {11+}\r\nFRED FOOBAR {7+}\r\nfat man\r\n",
                login::Arguments {
                    username: "FRED FOOBAR".to_string(),
                    password: "fat man".to_string(),
                },
            ),
        ] {
            assert_eq!(
                super::parse_login(
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
