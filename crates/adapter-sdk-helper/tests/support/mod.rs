//! A fake SDK helper: serves the helper protocol from a `FakeRemote`.
//!
//! It plays the role of the TypeScript helper so that the Rust side and the
//! protocol can be tested without Bun, the SDK or a Proton account.

// This is test code: a failed `unwrap` is supposed to abort the test.
#![allow(
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::missing_panics_doc
)]

use std::collections::HashMap;
use std::sync::Arc;

use protonsync_adapter_sdk_helper::protocol::{
    Chunk, DownloadHandle, Hello, PROTOCOL_VERSION, Request, Response, UploadHandle, WireError,
    WireEvent, WireEvents, WireNode, decode, encode,
};
use protonsync_adapter_sdk_helper::{Connection, HelperRemote};
use protonsync_application::ports::{EventCursor, RemoteError, RemoteStorage};
use protonsync_domain::node::{NodeId, NodeName};
use protonsync_domain::time::Timestamp;
use protonsync_testkit::FakeRemote;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

/// Root id of every `FakeRemote`.
pub const FAKE_ROOT: &str = "root";

/// A `HelperRemote` connected (in memory) to a fake helper on an empty storage.
#[allow(dead_code)] // Used by `contract.rs`, not by every test file.
pub fn helper_remote() -> HelperRemote {
    helper_remote_on(Arc::new(FakeRemote::new())).0
}

/// Like [`helper_remote`], but on a given storage; also returns the connection.
pub fn helper_remote_on(storage: Arc<FakeRemote>) -> (HelperRemote, Arc<Connection>) {
    let (ours, theirs) = tokio::io::duplex(4 * 1024 * 1024);
    let (their_reader, their_writer) = tokio::io::split(theirs);
    tokio::spawn(serve(storage, their_reader, their_writer));
    let (our_reader, our_writer) = tokio::io::split(ours);
    let connection = Arc::new(Connection::over(our_reader, our_writer));
    (
        HelperRemote::new(Arc::clone(&connection), NodeId::new(FAKE_ROOT)),
        connection,
    )
}

struct Upload {
    target: Target,
    size: u64,
    modified: Timestamp,
    data: Vec<u8>,
}

enum Target {
    NewFile { parent: NodeId, name: NodeName },
    Revision(NodeId),
}

#[derive(Default)]
struct Sessions {
    uploads: HashMap<String, Upload>,
    downloads: HashMap<String, (Vec<u8>, usize)>,
    next: u64,
    logged_in: bool,
    login_started: bool,
    login_waits: u32,
}

impl Sessions {
    fn handle(&mut self) -> String {
        self.next += 1;
        format!("h{}", self.next)
    }
}

/// Answers requests one after another until the input ends.
pub async fn serve<R, W>(storage: Arc<FakeRemote>, reader: R, mut writer: W)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut sessions = Sessions::default();
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let request: Request = serde_json::from_str(&line).unwrap();
        let response = match handle(&storage, &mut sessions, &request).await {
            Ok(value) => Response::success(request.id, value),
            Err(error) => Response::failure(request.id, WireError::from(&error)),
        };
        let mut out = serde_json::to_string(&response).unwrap();
        out.push('\n');
        if writer.write_all(out.as_bytes()).await.is_err() {
            return;
        }
    }
}

fn text(params: &Value, key: &str) -> String {
    params[key].as_str().unwrap().to_owned()
}

fn number(params: &Value, key: &str) -> u64 {
    params[key].as_u64().unwrap()
}

fn node_value(node: &protonsync_domain::node::Node) -> Value {
    serde_json::to_value(WireNode::from(node)).unwrap()
}

async fn handle(
    storage: &FakeRemote,
    sessions: &mut Sessions,
    request: &Request,
) -> Result<Value, RemoteError> {
    let p = &request.params;
    let id = |key: &str| NodeId::new(text(p, key));
    let name = || NodeName::new(text(p, "name")).unwrap();
    Ok(match request.method.as_str() {
        "hello" => serde_json::to_value(Hello {
            protocol: PROTOCOL_VERSION,
            sdk_version: "fake".into(),
        })
        .unwrap(),
        "node" => node_value(&storage.node(&id("node")).await?),
        "list_children" => {
            let mut children = storage.list_children(&id("folder")).await?;
            // The real SDK does not sort; the adapter must.
            children.reverse();
            serde_json::to_value(children.iter().map(WireNode::from).collect::<Vec<_>>()).unwrap()
        }
        "create_folder" => node_value(&storage.create_folder(&id("parent"), &name()).await?),
        "upload_open" | "revision_open" => {
            let target = if request.method == "upload_open" {
                Target::NewFile {
                    parent: id("parent"),
                    name: name(),
                }
            } else {
                Target::Revision(id("file"))
            };
            let handle = sessions.handle();
            sessions.uploads.insert(
                handle.clone(),
                Upload {
                    target,
                    size: number(p, "size"),
                    modified: Timestamp::from_unix_seconds(p["modified"].as_i64().unwrap()),
                    data: Vec::new(),
                },
            );
            serde_json::to_value(UploadHandle { upload: handle }).unwrap()
        }
        "upload_write" => {
            let upload = sessions.uploads.get_mut(&text(p, "upload")).unwrap();
            upload.data.extend(decode(&text(p, "data"))?);
            Value::Null
        }
        "upload_commit" => {
            let upload = sessions.uploads.remove(&text(p, "upload")).unwrap();
            let content = upload.data.as_slice();
            let node = match upload.target {
                Target::NewFile { parent, name } => {
                    storage
                        .upload_file(&parent, &name, content, upload.size, upload.modified)
                        .await?
                }
                Target::Revision(file) => {
                    storage
                        .upload_revision(&file, content, upload.size, upload.modified)
                        .await?
                }
            };
            node_value(&node)
        }
        "upload_abort" => {
            sessions.uploads.remove(&text(p, "upload"));
            Value::Null
        }
        "download_open" => {
            let mut content = Vec::new();
            storage.download_to(&id("file"), &mut content).await?;
            let handle = sessions.handle();
            sessions.downloads.insert(handle.clone(), (content, 0));
            serde_json::to_value(DownloadHandle { download: handle }).unwrap()
        }
        "download_read" => {
            let (content, position) = sessions.downloads.get_mut(&text(p, "download")).unwrap();
            // Small pieces, so that the adapter has to loop.
            let max = usize::try_from(number(p, "max")).unwrap().min(3);
            let end = (*position + max).min(content.len());
            let data = encode(&content[*position..end]);
            *position = end;
            serde_json::to_value(Chunk {
                data,
                eof: end == content.len(),
            })
            .unwrap()
        }
        "download_close" => {
            sessions.downloads.remove(&text(p, "download"));
            Value::Null
        }
        "download_range" => {
            let bytes = storage
                .download_range(&id("file"), number(p, "offset"), number(p, "length"))
                .await?;
            serde_json::to_value(Chunk {
                data: encode(&bytes),
                eof: false,
            })
            .unwrap()
        }
        "rename" => node_value(&storage.rename(&id("node"), &name()).await?),
        "move" => node_value(&storage.move_node(&id("node"), &id("parent")).await?),
        "trash" => {
            storage.trash(&id("node")).await?;
            Value::Null
        }
        "restore" => node_value(&storage.restore(&id("node")).await?),
        "delete" => {
            storage.delete_permanently(&id("node")).await?;
            Value::Null
        }
        "events_since" => {
            let cursor = p["cursor"].as_str().map(EventCursor::new);
            let (events, cursor) = storage.events_since(cursor.as_ref()).await?;
            serde_json::to_value(WireEvents {
                events: events.iter().map(WireEvent::from).collect(),
                cursor: cursor.as_str().to_owned(),
            })
            .unwrap()
        }
        "session" => json!({ "logged_in": sessions.logged_in,
                             "user": sessions.logged_in.then_some("tester@example.invalid") }),
        "login_web_start" => {
            sessions.login_started = true;
            json!({ "url": "https://account.example.invalid/desktop/login" })
        }
        "login_web_wait" => {
            if !sessions.login_started {
                return Err(RemoteError::Rejected("no browser login started".into()));
            }
            // The first wait finds the login still open, the second one done.
            sessions.login_waits += 1;
            if sessions.login_waits == 1 {
                json!({ "done": false, "user": null })
            } else {
                sessions.logged_in = true;
                sessions.login_started = false;
                json!({ "done": true, "user": "tester@example.invalid" })
            }
        }
        "logout" => {
            sessions.logged_in = false;
            Value::Null
        }
        "my_files_root" => node_value(&storage.root_folder().await?),
        "shared_with_me" => {
            let children = storage.list_children(&NodeId::new(FAKE_ROOT)).await?;
            serde_json::to_value(
                children
                    .iter()
                    .filter(|node| node.name.as_str().starts_with("shared-"))
                    .map(WireNode::from)
                    .collect::<Vec<_>>(),
            )
            .unwrap()
        }
        other => {
            return Err(RemoteError::Rejected(format!("unknown method {other}")));
        }
    })
    .map(|value| if value.is_null() { json!(null) } else { value })
}
