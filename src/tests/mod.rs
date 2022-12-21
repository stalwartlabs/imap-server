/*
 * Copyright (c) 2020-2022, Stalwart Labs Ltd.
 *
 * This file is part of the Stalwart IMAP Server.
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of
 * the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 * in the LICENSE file at the top-level directory of this distribution.
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/>.
 *
 * You can be released from the requirements of the AGPLv3 license by
 * purchasing a commercial license. Please contact licensing@stalw.art
 * for more details.
*/

pub mod acl;
pub mod append;
pub mod basic;
pub mod condstore;
pub mod copy_move;
pub mod fetch;
pub mod idle;
pub mod mailbox;
pub mod managesieve;
pub mod search;
pub mod store;
pub mod thread;

use std::{path::PathBuf, time::Duration};

use ahash::AHashMap;
use jmap_client::client::Client;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines, ReadHalf, WriteHalf},
    net::TcpStream,
};

use crate::{
    core::{env_settings::EnvSettings, ResponseType},
    start_imap_server,
};

#[tokio::test]
#[ignore]
pub async fn imap_tests() {
    // Prepare settings
    let (settings, temp_dir) = init_settings(true);

    // Start server
    tokio::spawn(async move {
        start_imap_server(settings).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Tests expect Stalwart JMAP server to be running on http://127.0.0.1:8080
    let jmap = Client::new()
        .credentials(("admin", "changeme"))
        .connect("http://127.0.0.1:8080")
        .await
        .unwrap();

    // Create test acounts
    jmap.domain_create("example.com").await.unwrap();
    jmap.individual_create("jdoe@example.com", "secret", "John Doe")
        .await
        .unwrap();
    jmap.individual_create("jane.smith@example.com", "secret", "Jane Smith")
        .await
        .unwrap();
    jmap.individual_create("foobar@example.com", "secret", "Bill Foobar")
        .await
        .unwrap();

    // Connect to IMAP server
    let mut imap_check = ImapConnection::connect(b"_y ").await;
    let mut imap = ImapConnection::connect(b"_x ").await;
    for imap in [&mut imap, &mut imap_check] {
        imap.assert_read(Type::Untagged, ResponseType::Ok).await;
    }

    // Unauthenticated tests
    basic::test(&mut imap, &mut imap_check).await;

    // Login
    for imap in [&mut imap, &mut imap_check] {
        imap.send("AUTHENTICATE PLAIN {32+}\r\nAGpkb2VAZXhhbXBsZS5jb20Ac2VjcmV0")
            .await;
        imap.assert_read(Type::Tagged, ResponseType::Ok).await;
    }

    // Delete folders
    for mailbox in ["Drafts", "Junk Mail", "Sent Items"] {
        imap.send(&format!("DELETE \"{}\"", mailbox)).await;
        imap.assert_read(Type::Tagged, ResponseType::Ok).await;
    }

    mailbox::test(&mut imap, &mut imap_check).await;
    append::test(&mut imap, &mut imap_check).await;
    search::test(&mut imap, &mut imap_check).await;
    fetch::test(&mut imap, &mut imap_check).await;
    store::test(&mut imap, &mut imap_check).await;
    copy_move::test(&mut imap, &mut imap_check).await;
    thread::test(&mut imap, &mut imap_check).await;
    idle::test(&mut imap, &mut imap_check).await;
    condstore::test(&mut imap, &mut imap_check).await;
    acl::test(&mut imap, &mut imap_check).await;

    // Logout
    for imap in [&mut imap, &mut imap_check] {
        imap.send("UNAUTHENTICATE").await;
        imap.assert_read(Type::Tagged, ResponseType::Ok).await;

        imap.send("LOGOUT").await;
        imap.assert_read(Type::Untagged, ResponseType::Bye).await;
    }

    // Run ManageSieve tests
    managesieve::test().await;

    // Delete temporary directory
    if temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir).unwrap();
    }
}

pub struct ImapConnection {
    tag: &'static [u8],
    reader: Lines<BufReader<ReadHalf<TcpStream>>>,
    writer: WriteHalf<TcpStream>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Tagged,
    Untagged,
    Continuation,
    Status,
}

impl ImapConnection {
    pub async fn connect(tag: &'static [u8]) -> Self {
        let (reader, writer) =
            tokio::io::split(TcpStream::connect("127.0.0.1:9991").await.unwrap());
        ImapConnection {
            tag,
            reader: BufReader::new(reader).lines(),
            writer,
        }
    }

    pub async fn assert_read(&mut self, t: Type, rt: ResponseType) -> Vec<String> {
        let lines = self.read(t).await;
        let mut buf = Vec::with_capacity(10);
        buf.extend_from_slice(match t {
            Type::Tagged => self.tag,
            Type::Untagged | Type::Status => b"* ",
            Type::Continuation => b"+ ",
        });
        if !matches!(t, Type::Continuation | Type::Status) {
            rt.serialize(&mut buf);
        }
        if lines
            .last()
            .unwrap()
            .starts_with(&String::from_utf8(buf).unwrap())
        {
            lines
        } else {
            panic!("Expected {:?}/{:?} from server but got: {:?}", t, rt, lines);
        }
    }

    pub async fn read(&mut self, t: Type) -> Vec<String> {
        let mut lines = Vec::new();
        loop {
            match tokio::time::timeout(Duration::from_millis(1500), self.reader.next_line()).await {
                Ok(Ok(Some(line))) => {
                    let is_done = line.starts_with(match t {
                        Type::Tagged => std::str::from_utf8(self.tag).unwrap(),
                        Type::Untagged | Type::Status => "* ",
                        Type::Continuation => "+ ",
                    });
                    println!("<- {:?}", line);
                    lines.push(line);
                    if is_done {
                        return lines;
                    }
                }
                Ok(Ok(None)) => {
                    panic!("Invalid response: {:?}.", lines);
                }
                Ok(Err(err)) => {
                    panic!("Connection broken: {} ({:?})", err, lines);
                }
                Err(_) => panic!("Timeout while waiting for server response: {:?}", lines),
            }
        }
    }

    pub async fn send(&mut self, text: &str) {
        println!("-> {}{:?}", std::str::from_utf8(self.tag).unwrap(), text);
        self.writer.write_all(self.tag).await.unwrap();
        self.writer.write_all(text.as_bytes()).await.unwrap();
        self.writer.write_all(b"\r\n").await.unwrap();
    }

    pub async fn send_untagged(&mut self, text: &str) {
        println!("-> {:?}", text);
        self.writer.write_all(text.as_bytes()).await.unwrap();
        self.writer.write_all(b"\r\n").await.unwrap();
    }

    pub async fn send_raw(&mut self, text: &str) {
        println!("-> {:?}", text);
        self.writer.write_all(text.as_bytes()).await.unwrap();
    }
}

pub trait AssertResult: Sized {
    fn assert_folders<'x>(
        self,
        expected: impl IntoIterator<Item = (&'x str, impl IntoIterator<Item = &'x str>)>,
        match_all: bool,
    ) -> Self;

    fn assert_response_code(self, code: &str) -> Self;
    fn assert_contains(self, text: &str) -> Self;
    fn assert_count(self, text: &str, occurences: usize) -> Self;
    fn assert_equals(self, text: &str) -> Self;
    fn into_response_code(self) -> String;
    fn into_highest_modseq(self) -> String;
    fn into_uid_validity(self) -> String;
    fn into_append_uid(self) -> String;
    fn into_copy_uid(self) -> String;
    fn into_modseq(self) -> String;
}

impl AssertResult for Vec<String> {
    fn assert_folders<'x>(
        self,
        expected: impl IntoIterator<Item = (&'x str, impl IntoIterator<Item = &'x str>)>,
        match_all: bool,
    ) -> Self {
        let mut match_count = 0;
        'outer: for (mailbox_name, flags) in expected.into_iter() {
            for result in self.iter() {
                if result.contains(&format!("\"{}\"", mailbox_name)) {
                    for flag in flags {
                        if !flag.is_empty() && !result.contains(flag) {
                            panic!("Expected mailbox {} to have flag {}", mailbox_name, flag);
                        }
                    }
                    match_count += 1;
                    continue 'outer;
                }
            }
            panic!("Mailbox {} is not present.", mailbox_name);
        }
        if match_all && match_count != self.len() - 1 {
            panic!(
                "Expected {} mailboxes, but got {}",
                match_count,
                self.len() - 1
            );
        }
        self
    }

    fn assert_response_code(self, code: &str) -> Self {
        if !self.last().unwrap().contains(&format!("[{}]", code)) {
            panic!(
                "Response code {:?} not found, got {:?}",
                code,
                self.last().unwrap()
            );
        }
        self
    }

    fn assert_contains(self, text: &str) -> Self {
        for line in &self {
            if line.contains(text) {
                return self;
            }
        }
        panic!("Expected response to contain {:?}, got {:?}", text, self);
    }

    fn assert_count(self, text: &str, occurences: usize) -> Self {
        assert_eq!(
            self.iter().filter(|l| l.contains(text)).count(),
            occurences,
            "Expected {} occurrences of {:?}, found {}.",
            occurences,
            text,
            self.iter().filter(|l| l.contains(text)).count()
        );
        self
    }

    fn assert_equals(self, text: &str) -> Self {
        for line in &self {
            if line == text {
                return self;
            }
        }
        panic!("Expected response to be {:?}, got {:?}", text, self);
    }

    fn into_response_code(self) -> String {
        if let Some((_, code)) = self.last().unwrap().split_once('[') {
            if let Some((code, _)) = code.split_once(']') {
                return code.to_string();
            }
        }
        panic!("No response code found in {:?}", self.last().unwrap());
    }

    fn into_append_uid(self) -> String {
        if let Some((_, code)) = self.last().unwrap().split_once("[APPENDUID ") {
            if let Some((code, _)) = code.split_once(']') {
                if let Some((_, uid)) = code.split_once(' ') {
                    return uid.to_string();
                }
            }
        }
        panic!("No APPENDUID found in {:?}", self.last().unwrap());
    }

    fn into_copy_uid(self) -> String {
        for line in &self {
            if let Some((_, code)) = line.split_once("[COPYUID ") {
                if let Some((code, _)) = code.split_once(']') {
                    if let Some((_, uid)) = code.split_once(' ') {
                        return uid.to_string();
                    }
                }
            }
        }
        panic!("No COPYUID found in {:?}", self);
    }

    fn into_highest_modseq(self) -> String {
        for line in &self {
            if let Some((_, value)) = line.split_once("HIGHESTMODSEQ ") {
                if let Some((value, _)) = value.split_once(']') {
                    return value.to_string();
                } else if let Some((value, _)) = value.split_once(')') {
                    return value.to_string();
                } else {
                    panic!("No HIGHESTMODSEQ delimiter found in {:?}", line);
                }
            }
        }
        panic!("No HIGHESTMODSEQ entries found in {:?}", self);
    }

    fn into_modseq(self) -> String {
        for line in &self {
            if let Some((_, value)) = line.split_once("MODSEQ (") {
                if let Some((value, _)) = value.split_once(')') {
                    return value.to_string();
                } else {
                    panic!("No MODSEQ delimiter found in {:?}", line);
                }
            }
        }
        panic!("No MODSEQ entries found in {:?}", self);
    }

    fn into_uid_validity(self) -> String {
        for line in &self {
            if let Some((_, value)) = line.split_once("UIDVALIDITY ") {
                if let Some((value, _)) = value.split_once(']') {
                    return value.to_string();
                } else if let Some((value, _)) = value.split_once(')') {
                    return value.to_string();
                } else {
                    panic!("No UIDVALIDITY delimiter found in {:?}", line);
                }
            }
        }
        panic!("No UIDVALIDITY entries found in {:?}", self);
    }
}

fn resources_dir() -> PathBuf {
    let mut resources = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    resources.push("src");
    resources.push("tests");
    resources.push("resources");
    resources
}

pub fn init_settings(delete_if_exists: bool) -> (EnvSettings, PathBuf) {
    let mut temp_dir = std::env::temp_dir();
    temp_dir.push("stalwart-imap-test");

    if delete_if_exists && temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    let resources = resources_dir();
    let mut key_path = resources.clone();
    let mut cert_path = resources;
    key_path.push("key.pem");
    cert_path.push("cert.pem");

    (
        EnvSettings {
            args: AHashMap::from_iter(
                vec![
                    (
                        "cache-dir".to_string(),
                        temp_dir.to_str().unwrap().to_string(),
                    ),
                    (
                        "key-path".to_string(),
                        key_path.to_str().unwrap().to_string(),
                    ),
                    (
                        "cert-path".to_string(),
                        cert_path.to_str().unwrap().to_string(),
                    ),
                    ("jmap-url".to_string(), "http://127.0.0.1:8080".to_string()),
                    ("bind-addr".to_string(), "127.0.0.1".to_string()),
                    ("bind-port".to_string(), "9991".to_string()),
                    ("bind-port-tls".to_string(), "9992".to_string()),
                    ("bind-port-sieve".to_string(), "4190".to_string()),
                    ("log-level".to_string(), "error".to_string()),
                ]
                .into_iter(),
            ),
        },
        temp_dir,
    )
}

pub fn destroy_temp_dir(temp_dir: PathBuf) {
    std::fs::remove_dir_all(temp_dir).unwrap();
}
