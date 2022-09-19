# Stalwart IMAP Server

[![Test](https://github.com/stalwartlabs/imap-server/actions/workflows/test.yml/badge.svg)](https://github.com/stalwartlabs/imap-server/actions/workflows/test.yml)
[![Build](https://github.com/stalwartlabs/imap-server/actions/workflows/build.yml/badge.svg)](https://github.com/stalwartlabs/imap-server/actions/workflows/build.yml)
[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)
[![](https://img.shields.io/discord/923615863037390889?label=Chat)](https://discord.gg/jVAuShSdNZ)
[![](https://img.shields.io/twitter/follow/stalwartlabs?style=flat)](https://twitter.com/stalwartlabs)

Stalwart IMAP is an open-source Internet Message Access Protocol server designed to be secure, fast, robust and scalable.
A JSON Meta Application Protocol (JMAP) backend such as [Stalwart JMAP](https://github.com/stalwartlabs/jmap-server) is required to use Stalwart IMAP (in other words, Stalwart
IMAP is an imap4-to-jmap proxy).

Key features:

- **IMAP4** full compliance and support for multiple extensions:
  - IMAP4rev2 ([RFC 9051](https://datatracker.ietf.org/doc/html/rfc9051))
  - IMAP4rev1 ([RFC 3501](https://datatracker.ietf.org/doc/html/rfc3501)) 
  - Access Control Lists (ACL) ([RFC 4314](https://datatracker.ietf.org/doc/html/rfc4314))
  - Conditional Store and Quick Resynchronization ([RFC 7162](https://datatracker.ietf.org/doc/html/rfc7162))
  - SORT and THREAD ([RFC 5256](https://datatracker.ietf.org/doc/html/rfc5256))
  - Message Preview Generation ([RFC 8970](https://datatracker.ietf.org/doc/html/rfc8970))
  - And [many other extensions](https://stalw.art/imap/development/rfc/#imap4-extensions)...
- **JMAP** backend:
  - Proxies IMAP4 requests to JMAP requests.
  - High-availability and fault-tolerance support when using a [Stalwart JMAP](https://github.com/stalwartlabs/jmap-server) backend.
  - Full compliance with [JMAP Core](https://datatracker.ietf.org/doc/html/rfc8620) and [JMAP Mail](https://datatracker.ietf.org/doc/html/rfc8621).
- **Secure**:
  - OAuth 2.0 [authorization code](https://www.rfc-editor.org/rfc/rfc8628) and [device authorization](https://www.rfc-editor.org/rfc/rfc8628) flows.
  - Rate limiting.
  - Memory safe (thanks to Rust).

## Get Started

Install Stalwart IMAP on your server by following the instructions for your platform:

- [Linux / MacOS](https://stalw.art/imap/get-started/linux/)
- [Windows](https://stalw.art/imap/get-started/windows/)
- [Docker](https://stalw.art/imap/get-started/docker/)

You may also [compile Stalwart IMAP from the source](https://stalw.art/imap/development/compile/).

## Support

If you are having problems running Stalwart IMAP, you found a bug or just have a question,
do not hesitate to reach us on [Github Discussions](https://github.com/stalwartlabs/imap-server/discussions),
[Reddit](https://www.reddit.com/r/stalwartlabs) or [Discord](https://discord.gg/jVAuShSdNZ).
Additionally you may become a sponsor to obtain priority support from Stalwart Labs Ltd.

## Documentation

Table of Contents

- Get Started
  - [Linux / MacOS](https://stalw.art/imap/get-started/linux/)
  - [Windows](https://stalw.art/imap/get-started/windows/)
  - [Docker](https://stalw.art/imap/get-started/docker/)
- Configuration
  - [Overview](https://stalw.art/imap/configure/overview/)
  - [IMAP Server](https://stalw.art/imap/configure/imap/)
  - [JMAP Proxy](https://stalw.art/imap/configure/proxy/)
  - [Cache](https://stalw.art/imap/configure/cache/)
- Development
  - [Compiling](https://stalw.art/imap/development/compile/)
  - [Tests](https://stalw.art/imap/development/test/)
  - [RFCs conformed](https://stalw.art/imap/development/rfc/)


## Roadmap

Support for the following IMAP extensions is planned for Stalwart IMAP:

- [RFC 2087 - IMAP4 QUOTA extension](https://datatracker.ietf.org/doc/html/rfc2087)
- [RFC 2192 - IMAP URL Scheme](https://datatracker.ietf.org/doc/html/rfc2192)
- [RFC 4467 - URLAUTH Extension](https://datatracker.ietf.org/doc/html/rfc4467)
- [RFC 4469 - IMAP CATENATE Extension](https://datatracker.ietf.org/doc/html/rfc4469)
- [RFC 4978 - IMAP COMPRESS Extension](https://datatracker.ietf.org/doc/html/rfc4978)
- [RFC 5255 - IMAP Internationalization](https://datatracker.ietf.org/doc/html/rfc5255)
- [RFC 5465 - IMAP NOTIFY Extension](https://datatracker.ietf.org/doc/html/rfc5465)
- [RFC 5524 - Extended URLFETCH for Binary and Converted Parts](https://datatracker.ietf.org/doc/html/rfc5524)
- [RFC 6785 - Support for IMAP Events in Sieve](https://datatracker.ietf.org/doc/html/rfc6785)

## Testing

### Base tests

The base tests perform protocol compliance tests as well as basic functionality testing on 
different functions across the Stalwart IMAP code base. 
To run the base test suite execute:

```bash
cargo test
```

### IMAP4 tests

The IMAP test suite performs a full server functionaly test including compliance to the IMAP4rev2/rev1
protocols and its extensions. To run these tests a blank Stalwart JMAP installation is required to be running at
``http://127.0.0.1:8080``.

To run the IMAP test suite execute:

```bash
cargo test imap_tests -- --ignored
```

### Third-party tests

Stalwart IMAP's protocol compliance may be also tested with Dovecot's ImapTest:

- Download [ImapTest](https://www.imapwiki.org/ImapTest/Installation).
- Start a blank Stalwart JMAP instance on ``http://127.0.0.1:8080``.
- Create a test account.
- Run the compliance tests as follows:
    ```
    ./imaptest host=<IMAP_HOSTNAME> port=<IMAP_PORT> \
            user=<JMAP_ACCOUNT> pass=<JMAP_ACCOUNT_SECRET> auth=100 \
            test=<PATH_TO_REPO>/src/tests/resources/imap-test/
    ```

Note: The tests distributed with ImapTest were slightly modified to support the
IMAP4rev2 specification.

### Stress tests

Stress testing Stalwart IMAP can be done with Dovecot's ImapTest:

- Download [ImapTest](https://www.imapwiki.org/ImapTest/Installation).
- Start a blank Stalwart JMAP instance on ``http://127.0.0.1:8080``.
- Create at least 3 test accounts, all using the same password. Store the account names in a file, one account per line.
- Run the stress tests as follows:
    ```
    ./imaptest host=<IMAP_HOSTNAME> port=<IMAP_PORT> \
            userfile=<PATH_TO_ACCOUNT_NAMES_FILE> \
            pass=<JMAP_ACCOUNT_SECRET> \
            mbox=<PATH_TO_TEST_MBOX> \
            auth=100
    ```

### Fuzz

To fuzz Stalwart IMAP server with `cargo-fuzz` execute:

```bash
 $ cargo +nightly fuzz run imap_server
```

## License

Licensed under the terms of the [GNU Affero General Public License](https://www.gnu.org/licenses/agpl-3.0.en.html) as published by
the Free Software Foundation, either version 3 of the License, or (at your option) any later version.
See [LICENSE](LICENSE) for more details.

You can be released from the requirements of the AGPLv3 license by purchasing
a commercial license. Please contact licensing@stalw.art for more details.
  
## Copyright

Copyright (C) 2020-2022, Stalwart Labs Ltd.

