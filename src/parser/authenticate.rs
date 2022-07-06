use crate::{
    core::receiver::Request,
    protocol::authenticate::{self, Mechanism},
};

impl Request {
    pub fn parse_authenticate(self) -> crate::core::Result<authenticate::Arguments> {
        if !self.tokens.is_empty() {
            let mut tokens = self.tokens.into_iter();
            Ok(authenticate::Arguments {
                mechanism: Mechanism::parse(&tokens.next().unwrap().unwrap_bytes())
                    .map_err(|v| (self.tag.as_str(), v))?,
                params: tokens
                    .into_iter()
                    .filter_map(|token| token.unwrap_string().ok())
                    .collect(),
                tag: self.tag,
            })
        } else {
            Err(self.into_error("Authentication mechanism missing."))
        }
    }
}

impl Mechanism {
    pub fn parse(value: &[u8]) -> super::Result<Self> {
        if value.eq_ignore_ascii_case(b"PLAIN") {
            Ok(Self::Plain)
        } else if value.eq_ignore_ascii_case(b"CRAM-MD5") {
            Ok(Self::CramMd5)
        } else if value.eq_ignore_ascii_case(b"DIGEST-MD5") {
            Ok(Self::DigestMd5)
        } else if value.eq_ignore_ascii_case(b"SCRAM-SHA-1") {
            Ok(Self::ScramSha1)
        } else if value.eq_ignore_ascii_case(b"SCRAM-SHA-256") {
            Ok(Self::ScramSha256)
        } else if value.eq_ignore_ascii_case(b"APOP") {
            Ok(Self::Apop)
        } else if value.eq_ignore_ascii_case(b"NTLM") {
            Ok(Self::Ntlm)
        } else if value.eq_ignore_ascii_case(b"GSSAPI") {
            Ok(Self::Gssapi)
        } else if value.eq_ignore_ascii_case(b"ANONYMOUS") {
            Ok(Self::Anonymous)
        } else if value.eq_ignore_ascii_case(b"EXTERNAL") {
            Ok(Self::External)
        } else if value.eq_ignore_ascii_case(b"OAUTHBEARER") {
            Ok(Self::OAuthBearer)
        } else if value.eq_ignore_ascii_case(b"XOAUTH2") {
            Ok(Self::XOauth2)
        } else {
            Err(format!(
                "Unsupported mechanism '{}'.",
                String::from_utf8_lossy(value)
            )
            .into())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        core::receiver::Receiver,
        protocol::authenticate::{self, Mechanism},
    };

    #[test]
    fn parse_authenticate() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                "a002 AUTHENTICATE \"EXTERNAL\" {16+}\r\nfred@example.com\r\n",
                authenticate::Arguments {
                    tag: "a002".to_string(),
                    mechanism: Mechanism::External,
                    params: vec!["fred@example.com".to_string()],
                },
            ),
            (
                "A01 AUTHENTICATE PLAIN\r\n",
                authenticate::Arguments {
                    tag: "A01".to_string(),
                    mechanism: Mechanism::Plain,
                    params: vec![],
                },
            ),
        ] {
            assert_eq!(
                receiver
                    .parse(&mut command.as_bytes().iter())
                    .unwrap()
                    .parse_authenticate()
                    .unwrap(),
                arguments
            );
        }
    }
}
