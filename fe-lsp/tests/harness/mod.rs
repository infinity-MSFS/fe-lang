//! A real server, driven over an in-memory connection.
//!
//! Nothing here stubs the server out: `Server::run` is running on a thread with
//! the same message loop the editors talk to, so a test exercises the protocol
//! rather than the functions behind it.

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use lsp_types::*;

pub struct Harness {
    client: Connection,
    root: PathBuf,
    next_id: AtomicU64,
    thread: Option<std::thread::JoinHandle<()>>,
    /// The most recent diagnostics seen for each file. The server publishes for
    /// every file it knows about whenever anything changes, so the useful
    /// question is not "what came next" but "what does it currently say".
    latest: RefCell<HashMap<Uri, Vec<Diagnostic>>>,
}

/// The example aircraft's manifest, so tests check against the same registry the
/// examples are written for.
pub const DC10_MANIFEST: &str = include_str!("../../../examples/dc10/fe.toml");

impl Harness {
    /// A project containing `files`, plus `fe.toml` unless `manifest` is `None`.
    pub fn new(files: &[(&str, &str)], manifest: Option<&str>) -> Harness {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "fe-lsp-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();

        if let Some(manifest) = manifest {
            std::fs::write(root.join("fe.toml"), manifest).unwrap();
        }
        for (name, text) in files {
            let path = root.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, text).unwrap();
        }

        let (server, client) = Connection::memory();
        let params = InitializeParams {
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: uri_of(&root),
                name: "test".to_string(),
            }]),
            ..Default::default()
        };
        let thread = std::thread::spawn(move || {
            fe_lsp::Server::new(server, params).run().unwrap();
        });

        Harness {
            client,
            root,
            next_id: AtomicU64::new(1),
            thread: Some(thread),
            latest: RefCell::new(HashMap::new()),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn uri(&self, name: &str) -> Uri {
        uri_of(&self.root.join(name))
    }

    pub fn write(&self, name: &str, text: &str) {
        std::fs::write(self.root.join(name), text).unwrap();
    }

    pub fn remove(&self, name: &str) {
        let _ = std::fs::remove_file(self.root.join(name));
    }

    // ---------------------------------------------------------- talking

    pub fn notify<N: notification::Notification>(&self, params: N::Params) {
        self.client
            .sender
            .send(Message::Notification(Notification::new(
                N::METHOD.to_string(),
                params,
            )))
            .unwrap();
    }

    pub fn request<R: request::Request>(&self, params: R::Params) -> R::Result {
        let id = RequestId::from(self.next_id.fetch_add(1, Ordering::Relaxed) as i32);
        self.client
            .sender
            .send(Message::Request(Request::new(
                id.clone(),
                R::METHOD.to_string(),
                params,
            )))
            .unwrap();

        loop {
            match self.recv() {
                Message::Response(Response {
                    id: got,
                    response_result,
                }) if got == id => match response_result {
                    Ok(result) => return serde_json::from_value(result).unwrap(),
                    Err(error) => panic!("{} failed: {}", R::METHOD, error.message),
                },
                _ => {}
            }
        }
    }

    pub fn open(&self, name: &str, text: &str) {
        self.notify::<notification::DidOpenTextDocument>(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: self.uri(name),
                language_id: "fe".to_string(),
                version: 1,
                text: text.to_string(),
            },
        });
    }

    pub fn change(&self, name: &str, text: &str) {
        self.notify::<notification::DidChangeTextDocument>(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: self.uri(name),
                version: 2,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: text.to_string(),
            }],
        });
    }

    /// Tell the server a file changed on disk, as a client's watcher would.
    pub fn watched(&self, name: &str, typ: FileChangeType) {
        self.notify::<notification::DidChangeWatchedFiles>(DidChangeWatchedFilesParams {
            changes: vec![FileEvent {
                uri: self.uri(name),
                typ,
            }],
        });
    }

    /// Receive one message, recording anything worth remembering on the way.
    fn recv(&self) -> Message {
        let message = self
            .client
            .receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the server should answer");

        match &message {
            Message::Notification(notification)
                if notification.method == notification::PublishDiagnostics::METHOD =>
            {
                let params: PublishDiagnosticsParams =
                    serde_json::from_value(notification.params.clone()).unwrap();
                self.latest
                    .borrow_mut()
                    .insert(params.uri, params.diagnostics);
            }
            // The server registers file watchers at startup; answer so it is
            // not left waiting on a reply.
            Message::Request(request) => {
                self.client
                    .sender
                    .send(Message::Response(Response::new_ok(
                        request.id.clone(),
                        serde_json::Value::Null,
                    )))
                    .unwrap();
            }
            _ => {}
        }
        message
    }

    /// Wait until everything sent so far has been processed *and* any
    /// diagnostics it caused have been published.
    ///
    /// One round-trip is enough because the server publishes pending
    /// diagnostics before answering a request, so a reply cannot overtake the
    /// publish caused by a notification sent before it.
    fn sync(&self) {
        let _: Option<WorkspaceSymbolResponse> =
            self.request::<request::WorkspaceSymbolRequest>(WorkspaceSymbolParams {
                query: "\u{0}unlikely".to_string(),
                ..Default::default()
            });
    }

    /// What the server currently says about `name`.
    pub fn diagnostics(&self, name: &str) -> Vec<Diagnostic> {
        self.sync();
        self.latest
            .borrow()
            .get(&self.uri(name))
            .cloned()
            .unwrap_or_default()
    }

    /// Codes of the diagnostics for `name`.
    pub fn codes(&self, name: &str) -> Vec<String> {
        self.diagnostics(name)
            .into_iter()
            .filter_map(|d| match d.code {
                Some(NumberOrString::String(code)) => Some(code),
                _ => None,
            })
            .collect()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let id = RequestId::from(9999);
        let _ = self.client.sender.send(Message::Request(Request::new(
            id,
            request::Shutdown::METHOD.to_string(),
            (),
        )));
        let _ = self
            .client
            .sender
            .send(Message::Notification(Notification::new(
                notification::Exit::METHOD.to_string(),
                (),
            )));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The same conversion the server uses. A second implementation here would
/// only ever test itself: the URIs a test compares against have to be the ones
/// the server would produce, separators and all.
pub fn uri_of(path: &Path) -> Uri {
    fe_lsp::uri::from_path(path).unwrap()
}

pub fn position(line: u32, character: u32) -> Position {
    Position { line, character }
}

/// Where `needle` starts in `text`, as a position.
pub fn at(text: &str, needle: &str) -> Position {
    let offset = text
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in source"));
    let line = text[..offset].matches('\n').count() as u32;
    let line_start = text[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    position(line, (offset - line_start) as u32)
}
