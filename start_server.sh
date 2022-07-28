#!/bin/sh

RUST_LOG="imap_server=debug" CACHE_DIR="/tmp/stalwart-imap" JMAP_URL="http://127.0.0.1:8080/.well-known/jmap" BIND_PORT=9991 BIND_PORT_TLS=9992 CERT_PATH="./src/tests/resources/cert.pem" KEY_PATH="./src/tests/resources/key.pem" cargo run
#rm -Rf /tmp/imap-jmap-db ; RUST_LOG="jmap_server=debug" cargo run -- --db-path /tmp/imap-jmap-db
#rm -Rf /tmp/imap-jmap-db; cp -R /tmp/imap-jmap-db.empty /tmp/imap-jmap-db ; RUST_LOG="jmap_server=debug" cargo run -- --db-path /tmp/imap-jmap-db
