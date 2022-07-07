use std::{collections::HashMap, path::PathBuf, time::Duration};

use jmap_client::client::Client;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines, ReadHalf, WriteHalf},
    net::TcpStream,
};

use crate::{
    core::{env_settings::EnvSettings, ResponseType, StatusResponse},
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
    /*let jmap = Client::connect(
        "http://127.0.0.1:8080/.well-known/jmap",
        ("admin", "changeme"),
    )
    .await
    .unwrap();

    // Create test users
    jmap.individual_create("jdoe@example.com", "secret", "John Doe")
        .await
        .unwrap();*/

    // Connect to IMAP server
    let mut imap = ImapConnection::connect().await;
    imap.read_assert(Type::Untagged, ResponseType::Ok).await;

    // Test CAPABILITY
    imap.send("CAPABILITY").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;

    // Test NOOP
    imap.send("NOOP").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;

    // Login should be disabled
    imap.send("LOGIN jdoe@example.com secret").await;
    imap.read_assert(Type::Tagged, ResponseType::No).await;

    // Try logging in with wrong password
    /*imap.send("AUTHENTICATE PLAIN {24}").await;
        imap.read_assert(Type::Continuation, ResponseType::Ok).await;
        imap.send_untagged("AGJvYXR5AG1jYm9hdGZhY2U=").await;
        imap.read_assert(Type::Tagged, ResponseType::No).await;

        // Login with correct password
        imap.send("AUTHENTICATE PLAIN {32+}\r\nAGpkb2VAZXhhbXBsZS5jb20Ac2VjcmV0")
            .await;
        imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    */

    // Logout
    imap.send("LOGOUT").await;
    imap.read_assert(Type::Untagged, ResponseType::Bye).await;

    // Delete temporary directory
    if temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir).unwrap();
    }
}

struct ImapConnection {
    reader: Lines<BufReader<ReadHalf<TcpStream>>>,
    writer: WriteHalf<TcpStream>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Type {
    Tagged,
    Untagged,
    Continuation,
}

impl ImapConnection {
    pub async fn connect() -> Self {
        let (reader, writer) =
            tokio::io::split(TcpStream::connect("127.0.0.1:9991").await.unwrap());
        ImapConnection {
            reader: BufReader::new(reader).lines(),
            writer,
        }
    }

    pub async fn read_assert(&mut self, t: Type, rt: ResponseType) -> Vec<String> {
        let lines = self.read(t).await;
        let mut buf = Vec::with_capacity(10);
        buf.extend_from_slice(match t {
            Type::Tagged => b"_x ",
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
                        Type::Tagged => "_x ",
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
        println!("-> _x {:?}", text);
        self.writer.write_all(b"_x ").await.unwrap();
        self.writer.write_all(text.as_bytes()).await.unwrap();
        self.writer.write_all(b"\r\n").await.unwrap();
    }

    pub async fn send_untagged(&mut self, text: &str) {
        println!("-> {:?}", text);
        self.writer.write_all(text.as_bytes()).await.unwrap();
        self.writer.write_all(b"\r\n").await.unwrap();
    }
}

fn resources_dir() -> PathBuf {
    let mut resources = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    resources.push("src");
    resources.push("tests");
    resources.push("resources");
    resources
}

fn init_settings(delete_if_exists: bool) -> (EnvSettings, PathBuf) {
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
