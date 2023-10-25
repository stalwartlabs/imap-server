# Change Log

All notable changes to this project will be documented in this file. This project adheres to [Semantic Versioning](http://semver.org/).

## [0.4.0] - 2023-10-25

This version introduces some breaking changes in the configuration file. Please read the [UPGRADING.md](UPGRADING.md) file for more information.

## Added
- Built-in Spam and Phishing filter.
- Scheduled queries on some directory types.
- In-memory maps and lists containing glob or regex patterns.
- Remote retrieval of in-memory list/maps with fallback mechanisms.
- Macros and support for including files from TOML config files.

### Changed
- `config.toml` is now split in multiple TOML files for better organization.
- **BREAKING:** Configuration key prefix `jmap.sieve` (JMAP Sieve Interpreter) has been renamed to `sieve.untrusted`.
- **BREAKING:** Configuration key prefix `sieve` (SMTP Sieve Interpreter) has been renamed to `sieve.trusted`.

### Fixed

## [0.3.8] - 2023-09-19

## Added
- Journal logging support
- IMAP support for UTF8 APPEND

### Changed
- Replaced `rpgp` with `sequoia-pgp` due to rpgp bug.

### Fixed
- Fix: IMAP folders that contain a & can't be used (#90) 
- Fix: Ignore empty lines in IMAP requests

## [0.3.7] - 2023-09-05

## Added
- Option to disable IMAP All Messages folder (#68).

### Changed
 
### Fixed
- Invalid IMAP `FETCH` responses for non-UTF-8 messages (#70)
- Allow `STATUS` and `ACL` IMAP operations on virtual mailboxes.
- IMAP `SELECT QRESYNC` without specifying a UID causes panic (#67)

## [0.3.6] - 2023-08-29

## Added
- Arithmetic and logical expression evaluation in Sieve scripts.
- Support for storing query results in Sieve variables.
- Configurable protocol flags for Milter filters.

### Changed
 
### Fixed
- ManageSieve `PUTSCRIPT` should replace existing scripts.

## [0.3.5] - 2023-08-18

## Added
- TCP listener option `nodelay`.
 
### Changed
 
### Fixed

## [0.3.4] - 2023-08-09

## Added
 
### Changed
 
### Fixed
- Successful authentication requests should not count when rate limiting
- Case insensitive Inbox selection
- Automatically create Inbox for group accounts

## [0.3.3] - 2023-08-02

### Added
- Encryption at rest with **S/MIME** or **OpenPGP**.
- Support for referencing context variables from dynamic values.
 
### Changed
 
### Fixed
- Support for PKCS8v1 ED25519 keys (#20).

## [0.3.2] - 2023-07-28

### Added
- Sender and recipient address rewriting using regular expressions and sieve scripts.
- Subaddressing and catch-all addresses using regular expressions (#10).
 
### Changed
- Added CLI to Docker container (#19).
 
### Fixed
- Workaround for a bug in `sqlx` that caused SQL time-outs (#15).
- Support for ED25519 certificates in PEM files (#20). 
- Better handling of concurrent IMAP UID map modifications (#17).
- LDAP domain lookups from SMTP rules.

## [0.3.1] - 2023-07-22

### Added
 
### Changed
 
### Fixed
- Support for OpenLDAP password hashing schemes between curly brackets (#8). 
- Add CA certificates to Docker runtime (#5).

## [0.3.0] - 2023-07-16

### Added

### Changed
- Rewritten IMAP server to have direct access to the message store (no more IMAP proxy).
 
### Fixed

## [0.2.0] - 2022-10-31

### Added
- ManageSieve support.
- Added UTF8=ACCEPT (RFC 6855) support.

### Changed
 
### Fixed
- Fixed BODY[1] bug.

## [0.1.0] - 2022-09-15

Initial release.

