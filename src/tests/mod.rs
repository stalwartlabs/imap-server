pub mod append;
pub mod basic;
pub mod fetch;
pub mod mailbox;
pub mod search;

use std::{collections::HashMap, path::PathBuf, time::Duration};

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
pub async fn test_server() {
    // Prepare settings
    let (settings, temp_dir) = init_settings(true);

    // Start server
    tokio::spawn(async move {
        start_imap_server(settings).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Tests expect Stalwart JMAP server to be running on http://127.0.0.1:8080
    let jmap = Client::connect(
        "http://127.0.0.1:8080/.well-known/jmap",
        ("admin", "changeme"),
    )
    .await
    .unwrap();

    // Create test users
    /*jmap.domain_create("example.com").await.unwrap();
    jmap.individual_create("jdoe@example.com", "secret", "John Doe")
        .await
        .unwrap();*/

    // Connect to IMAP server
    let mut imap_check = ImapConnection::connect(b"_y ").await;
    let mut imap = ImapConnection::connect(b"_x ").await;
    for imap in [&mut imap, &mut imap_check] {
        imap.assert_read(Type::Untagged, ResponseType::Ok).await;
    }

    // Unauthenticated tests
    let comments = "remove";
    //basic::test(&mut imap, &mut imap_check).await;

    // Login
    for imap in [&mut imap, &mut imap_check] {
        imap.send("AUTHENTICATE PLAIN {32+}\r\nAGpkb2VAZXhhbXBsZS5jb20Ac2VjcmV0")
            .await;
        imap.assert_read(Type::Tagged, ResponseType::Ok).await;
    }

    //mailbox::test(&mut imap, &mut imap_check).await;
    //append::test(&mut imap, &mut imap_check).await;
    //search::test(&mut imap, &mut imap_check).await;
    append::test(&mut imap, &mut imap_check).await;

    // Logout
    for imap in [&mut imap, &mut imap_check] {
        imap.send("LOGOUT").await;
        imap.assert_read(Type::Untagged, ResponseType::Bye).await;
    }

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
            Type::Untagged => b"* ",
            Type::Continuation => b"+ ",
        });
        if t != Type::Continuation {
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
            match tokio::time::timeout(Duration::from_millis(500), self.reader.next_line()).await {
                Ok(Ok(Some(line))) => {
                    let is_done = line.starts_with(match t {
                        Type::Tagged => std::str::from_utf8(self.tag).unwrap(),
                        Type::Untagged => "* ",
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
}

pub trait AssertResult: Sized {
    fn assert_folders<'x>(
        self,
        expected: impl IntoIterator<Item = (&'x str, impl IntoIterator<Item = &'x str>)>,
        match_all: bool,
    ) -> Self;

    fn assert_response_code(self, code: &str) -> Self;
    fn assert_contains(self, text: &str) -> Self;
    fn assert_equals(self, text: &str) -> Self;
    fn get_response_code(&self) -> &str;
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

    fn assert_equals(self, text: &str) -> Self {
        for line in &self {
            if line == text {
                return self;
            }
        }
        panic!("Expected response to be {:?}, got {:?}", text, self);
    }

    fn get_response_code(&self) -> &str {
        if let Some((_, code)) = self.last().unwrap().split_once('[') {
            if let Some((code, _)) = code.split_once(']') {
                return code;
            }
        }
        panic!("No response code found in {:?}", self.last().unwrap());
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
            args: HashMap::from_iter(
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
                    (
                        "jmap-url".to_string(),
                        "http://127.0.0.1:8080/.well-known/jmap".to_string(),
                    ),
                    ("bind-addr".to_string(), "127.0.0.1".to_string()),
                    ("bind-port".to_string(), "9991".to_string()),
                    ("bind-port-tls".to_string(), "9992".to_string()),
                ]
                .into_iter(),
            ),
        },
        temp_dir,
    )
}

pub fn destroy_temp_dir(temp_dir: PathBuf) {
    std::fs::remove_dir_all(&temp_dir).unwrap();
}
