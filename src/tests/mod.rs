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
    jmap.domain_create("example.com").await.unwrap();
    jmap.individual_create("jdoe@example.com", "secret", "John Doe")
        .await
        .unwrap();

    // Connect to IMAP server
    let mut imap_check = ImapConnection::connect(b"_y ").await;
    let mut imap = ImapConnection::connect(b"_x ").await;
    for imap in [&mut imap, &mut imap_check] {
        imap.read_assert(Type::Untagged, ResponseType::Ok).await;
    }

    // Test CAPABILITY
    /*imap.send("CAPABILITY").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;

    // Test NOOP
    imap.send("NOOP").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;

    // Login should be disabled
    imap.send("LOGIN jdoe@example.com secret").await;
    imap.read_assert(Type::Tagged, ResponseType::No).await;

    // Try logging in with wrong password
    imap.send("AUTHENTICATE PLAIN {24}").await;
    imap.read_assert(Type::Continuation, ResponseType::Ok).await;
    imap.send_untagged("AGJvYXR5AG1jYm9hdGZhY2U=").await;
    imap.read_assert(Type::Tagged, ResponseType::No).await;*/

    // Login with correct password
    for imap in [&mut imap, &mut imap_check] {
        imap.send("AUTHENTICATE PLAIN {32+}\r\nAGpkb2VAZXhhbXBsZS5jb20Ac2VjcmV0")
            .await;
        imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    }

    // List folders
    imap.send("LIST \"\" \"*\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok)
        .await
        .assert_folders(
            [
                ("All Messages", ["NoInferiors"]),
                ("INBOX", [""]),
                ("Deleted Items", [""]),
            ],
            true,
        );

    // Create folders
    imap.send("CREATE \"Tofu\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    imap.send("CREATE \"Fruit\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    imap.send("CREATE \"Fruit/Apple\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    imap.send("CREATE \"Fruit/Apple/Green\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    for imap in [&mut imap, &mut imap_check] {
        imap.send("LIST \"\" \"*\"").await;
        imap.read_assert(Type::Tagged, ResponseType::Ok)
            .await
            .assert_folders(
                [
                    ("All Messages", ["NoInferiors"]),
                    ("INBOX", [""]),
                    ("Deleted Items", [""]),
                    ("Fruit", [""]),
                    ("Fruit/Apple", [""]),
                    ("Fruit/Apple/Green", [""]),
                    ("Tofu", [""]),
                ],
                true,
            );
    }

    // Folders under All Messages should not be allowed
    imap.send("CREATE \"All Messages/Untitled\"").await;
    imap.read_assert(Type::Tagged, ResponseType::No).await;

    // Enable IMAP4rev2
    for imap in [&mut imap, &mut imap_check] {
        imap.send("ENABLE IMAP4rev2").await;
        imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    }

    // Create missing parent folders
    imap.send("CREATE \"/Vegetable/Broccoli\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    imap.send("CREATE \" Cars/Electric /4 doors/ Red/\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    for imap in [&mut imap, &mut imap_check] {
        imap.send("LIST \"\" \"*\" RETURN (CHILDREN)").await;
        imap.read_assert(Type::Tagged, ResponseType::Ok)
            .await
            .assert_folders(
                [
                    ("All Messages", ["NoInferiors", "All"]),
                    ("INBOX", ["HasNoChildren", ""]),
                    ("Deleted Items", ["HasNoChildren", "Trash"]),
                    ("Cars/Electric/4 doors/Red", ["HasNoChildren", ""]),
                    ("Cars/Electric/4 doors", ["HasChildren", ""]),
                    ("Cars/Electric", ["HasChildren", ""]),
                    ("Cars", ["HasChildren", ""]),
                    ("Fruit", ["HasChildren", ""]),
                    ("Fruit/Apple", ["HasChildren", ""]),
                    ("Fruit/Apple/Green", ["HasNoChildren", ""]),
                    ("Vegetable", ["HasChildren", ""]),
                    ("Vegetable/Broccoli", ["HasNoChildren", ""]),
                    ("Tofu", ["HasNoChildren", ""]),
                ],
                true,
            );
    }

    // Rename folders
    imap.send("RENAME \"Fruit/Apple/Green\" \"Fruit/Apple/Red\"")
        .await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    imap.send("RENAME \"Cars\" \"Vehicles\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    imap.send("RENAME \"Vegetable/Broccoli\" \"Veggies/Green/Broccoli\"")
        .await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    imap.send("RENAME \"Tofu\" \"INBOX\"").await;
    imap.read_assert(Type::Tagged, ResponseType::No).await;
    imap.send("RENAME \"Tofu\" \"INBOX/Tofu\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    imap.send("RENAME \"Deleted Items\" \"Recycle Bin\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    for imap in [&mut imap, &mut imap_check] {
        imap.send("LIST \"\" \"*\" RETURN (CHILDREN)").await;
        imap.read_assert(Type::Tagged, ResponseType::Ok)
            .await
            .assert_folders(
                [
                    ("All Messages", ["NoInferiors", "All"]),
                    ("INBOX", ["HasChildren", ""]),
                    ("INBOX/Tofu", ["HasNoChildren", ""]),
                    ("Recycle Bin", ["HasNoChildren", "Trash"]),
                    ("Vehicles/Electric/4 doors/Red", ["HasNoChildren", ""]),
                    ("Vehicles/Electric/4 doors", ["HasChildren", ""]),
                    ("Vehicles/Electric", ["HasChildren", ""]),
                    ("Vehicles", ["HasChildren", ""]),
                    ("Fruit", ["HasChildren", ""]),
                    ("Fruit/Apple", ["HasChildren", ""]),
                    ("Fruit/Apple/Red", ["HasNoChildren", ""]),
                    ("Vegetable", ["HasNoChildren", ""]),
                    ("Veggies", ["HasChildren", ""]),
                    ("Veggies/Green", ["HasChildren", ""]),
                    ("Veggies/Green/Broccoli", ["HasNoChildren", ""]),
                ],
                true,
            );
    }

    // Delete folders
    imap.send("DELETE \"INBOX/Tofu\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    imap.send("DELETE \"Vegetable\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    imap.send("DELETE \"All Messages\"").await;
    imap.read_assert(Type::Tagged, ResponseType::No).await;
    imap.send("DELETE \"Vehicles\"").await;
    imap.read_assert(Type::Tagged, ResponseType::No).await;
    for imap in [&mut imap, &mut imap_check] {
        imap.send("LIST \"\" \"*\" RETURN (CHILDREN)").await;
        imap.read_assert(Type::Tagged, ResponseType::Ok)
            .await
            .assert_folders(
                [
                    ("All Messages", ["NoInferiors", "All"]),
                    ("INBOX", ["HasNoChildren", ""]),
                    ("Recycle Bin", ["HasNoChildren", "Trash"]),
                    ("Vehicles/Electric/4 doors/Red", ["HasNoChildren", ""]),
                    ("Vehicles/Electric/4 doors", ["HasChildren", ""]),
                    ("Vehicles/Electric", ["HasChildren", ""]),
                    ("Vehicles", ["HasChildren", ""]),
                    ("Fruit", ["HasChildren", ""]),
                    ("Fruit/Apple", ["HasChildren", ""]),
                    ("Fruit/Apple/Red", ["HasNoChildren", ""]),
                    ("Veggies", ["HasChildren", ""]),
                    ("Veggies/Green", ["HasChildren", ""]),
                    ("Veggies/Green/Broccoli", ["HasNoChildren", ""]),
                ],
                true,
            );
    }

    // Subscribe
    imap.send("SUBSCRIBE \"INBOX\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    imap.send("SUBSCRIBE \"Vehicles/Electric/4 doors/Red\"")
        .await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    for imap in [&mut imap, &mut imap_check] {
        imap.send("LIST \"\" \"*\" RETURN (SUBSCRIBED)").await;
        imap.read_assert(Type::Tagged, ResponseType::Ok)
            .await
            .assert_folders(
                [
                    ("All Messages", ["NoInferiors", "All"]),
                    ("INBOX", ["Subscribed", ""]),
                    ("Recycle Bin", ["", "Trash"]),
                    ("Vehicles/Electric/4 doors/Red", ["Subscribed", ""]),
                    ("Vehicles/Electric/4 doors", ["", ""]),
                    ("Vehicles/Electric", ["", ""]),
                    ("Vehicles", ["", ""]),
                    ("Fruit", ["", ""]),
                    ("Fruit/Apple", ["", ""]),
                    ("Fruit/Apple/Red", ["", ""]),
                    ("Veggies", ["", ""]),
                    ("Veggies/Green", ["", ""]),
                    ("Veggies/Green/Broccoli", ["", ""]),
                ],
                true,
            );
    }

    // Filter by subscribed including children
    imap.send("LIST (SUBSCRIBED) \"\" \"*\" RETURN (CHILDREN)")
        .await;
    imap.read_assert(Type::Tagged, ResponseType::Ok)
        .await
        .assert_folders(
            [
                ("INBOX", ["Subscribed", "HasNoChildren"]),
                (
                    "Vehicles/Electric/4 doors/Red",
                    ["Subscribed", "HasNoChildren"],
                ),
            ],
            true,
        );

    // Recursive match including children
    imap.send("LIST (SUBSCRIBED RECURSIVEMATCH) \"\" \"*\" RETURN (CHILDREN)")
        .await;
    imap.read_assert(Type::Tagged, ResponseType::Ok)
        .await
        .assert_folders(
            [
                ("INBOX", ["Subscribed", "HasNoChildren"]),
                (
                    "Vehicles/Electric/4 doors/Red",
                    ["Subscribed", "HasNoChildren"],
                ),
                (
                    "Vehicles/Electric/4 doors",
                    ["\"CHILDINFO\" (\"SUBSCRIBED\")", "HasChildren"],
                ),
                (
                    "Vehicles/Electric",
                    ["\"CHILDINFO\" (\"SUBSCRIBED\")", "HasChildren"],
                ),
                (
                    "Vehicles",
                    ["\"CHILDINFO\" (\"SUBSCRIBED\")", "HasChildren"],
                ),
            ],
            true,
        );

    // Imap4rev1 LSUB
    imap.send("LSUB \"\" \"*\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok)
        .await
        .assert_folders(
            [("INBOX", [""]), ("Vehicles/Electric/4 doors/Red", [""])],
            true,
        );

    // Unsubscribe
    imap.send("UNSUBSCRIBE \"Vehicles/Electric/4 doors/Red\"")
        .await;
    imap.read_assert(Type::Tagged, ResponseType::Ok).await;
    for imap in [&mut imap, &mut imap_check] {
        imap.send("LIST (SUBSCRIBED RECURSIVEMATCH) \"\" \"*\" RETURN (CHILDREN)")
            .await;
        imap.read_assert(Type::Tagged, ResponseType::Ok)
            .await
            .assert_folders([("INBOX", ["Subscribed", "HasNoChildren"])], true);
    }

    // LIST Filters
    imap.send("LIST \"\" \"%\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok)
        .await
        .assert_folders(
            [
                ("All Messages", [""]),
                ("INBOX", [""]),
                ("Recycle Bin", [""]),
                ("Vehicles", [""]),
                ("Fruit", [""]),
                ("Veggies", [""]),
            ],
            true,
        );

    imap.send("LIST \"\" \"*/Red\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok)
        .await
        .assert_folders(
            [
                ("Vehicles/Electric/4 doors/Red", [""]),
                ("Fruit/Apple/Red", [""]),
            ],
            true,
        );

    imap.send("LIST \"\" \"Fruit/*\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok)
        .await
        .assert_folders([("Fruit/Apple/Red", [""]), ("Fruit/Apple", [""])], true);

    imap.send("LIST \"\" \"Fruit/%\"").await;
    imap.read_assert(Type::Tagged, ResponseType::Ok)
        .await
        .assert_folders([("Fruit/Apple", [""])], true);

    // Logout
    for imap in [&mut imap, &mut imap_check] {
        imap.send("LOGOUT").await;
        imap.read_assert(Type::Untagged, ResponseType::Bye).await;
    }

    // Delete temporary directory
    if temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir).unwrap();
    }
}

struct ImapConnection {
    tag: &'static [u8],
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
    pub async fn connect(tag: &'static [u8]) -> Self {
        let (reader, writer) =
            tokio::io::split(TcpStream::connect("127.0.0.1:9991").await.unwrap());
        ImapConnection {
            tag,
            reader: BufReader::new(reader).lines(),
            writer,
        }
    }

    pub async fn read_assert(&mut self, t: Type, rt: ResponseType) -> Vec<String> {
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

pub trait AssertResult {
    fn assert_folders<'x>(
        &self,
        expected: impl IntoIterator<Item = (&'x str, impl IntoIterator<Item = &'x str>)>,
        match_all: bool,
    );
}

impl AssertResult for Vec<String> {
    fn assert_folders<'x>(
        &self,
        expected: impl IntoIterator<Item = (&'x str, impl IntoIterator<Item = &'x str>)>,
        match_all: bool,
    ) {
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
