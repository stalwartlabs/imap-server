#!/bin/sh

RUST_LOG="imap_server=debug" BIND_PORT=9991 BIND_PORT_TLS=9992 CERT_PATH="./src/tests/resources/cert.pem" KEY_PATH="./src/tests/resources/key.pem" cargo run
