use crate::{
    core::receiver::Request,
    protocol::{capability::Capability, enable},
};

pub fn parse_enable(request: Request) -> crate::core::Result<enable::Arguments> {
    let len = request.tokens.len();
    if len > 0 {
        let mut capabilities = Vec::with_capacity(len);
        for capability in request.tokens {
            capabilities.push(
                Capability::parse(&capability.unwrap_bytes())
                    .map_err(|v| (request.tag.as_str(), v))?,
            );
        }
        Ok(enable::Arguments { capabilities })
    } else {
        Err(request.into_error("Missing arguments."))
    }
}

impl Capability {
    pub fn parse(value: &[u8]) -> super::Result<Self> {
        if value.eq_ignore_ascii_case(b"IMAP4rev2") {
            Ok(Self::IMAP4rev2)
        } else if value.eq_ignore_ascii_case(b"STARTTLS") {
            Ok(Self::StartTLS)
        } else if value.eq_ignore_ascii_case(b"LOGINDISABLED") {
            Ok(Self::LoginDisabled)
        } else if value.eq_ignore_ascii_case(b"CONDSTORE") {
            Ok(Self::Condstore)
        } else {
            Err(format!(
                "Unsupported capability '{}'.",
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
        protocol::{capability::Capability, enable},
    };

    #[test]
    fn parse_enable() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [(
            "t2 ENABLE IMAP4rev2 CONDSTORE\r\n",
            enable::Arguments {
                capabilities: vec![Capability::IMAP4rev2, Capability::Condstore],
            },
        )] {
            assert_eq!(
                super::parse_enable(receiver.parse(&mut command.as_bytes().iter()).unwrap())
                    .unwrap(),
                arguments
            );
        }
    }
}
