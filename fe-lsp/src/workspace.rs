//! Every `.fe` file the project has, and the manifest that says what they may
//! mention.
//!
//! Analysis is always whole-project, never per-file. Procedure identifiers
//! share one flat namespace across every source compiled together, so `call`
//! resolution and duplicate detection are questions no single file can answer —
//! and a `.fe` file that is not open in the editor is still part of the answer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fe_project::{MANIFEST_NAME, Manifest, ManifestError};

use crate::line_index::LineIndex;

/// Directories that are never worth walking into.
const SKIP: &[&str] = &["target", "node_modules", ".git", ".svn", "out", "dist"];

#[derive(Clone, Debug)]
pub struct Document {
    pub text: String,
    pub index: LineIndex,
    /// `None` for a file read from disk; the client's version for an open one.
    pub version: Option<i32>,
}

impl Document {
    pub fn new(text: String, version: Option<i32>) -> Document {
        Document {
            index: LineIndex::new(&text),
            text,
            version,
        }
    }
}

/// How much the server is able to say about this project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// A manifest was found: names, types, control kinds and ranges are all
    /// checked, exactly as a build would check them.
    Semantic,
    /// No manifest. Only lexical and syntactic diagnostics are reported.
    ///
    /// Analysing against an empty registry instead would be worse than useless:
    /// every symbol in every file would be E0201, burying the real errors under
    /// a wall of red about names that are perfectly fine.
    SyntaxOnly { reason: String },
}

impl Mode {
    pub fn is_semantic(&self) -> bool {
        matches!(self, Mode::Semantic)
    }
}

pub struct Workspace {
    root: PathBuf,
    /// An explicit manifest path from configuration, if the user set one.
    manifest_override: Option<PathBuf>,
    manifest_path: Option<PathBuf>,
    manifest: Option<Manifest>,
    manifest_errors: Vec<ManifestError>,
    manifest_text: String,
    /// Files, keyed by path. Sorted iteration order is what makes `UnitId`
    /// assignment stable between analyses.
    files: BTreeMap<PathBuf, Document>,
    /// Paths the client has open. These take precedence over what is on disk,
    /// and are not dropped when a filesystem event says the file changed.
    open: Vec<PathBuf>,
}

impl Workspace {
    pub fn new(root: PathBuf, manifest_override: Option<PathBuf>) -> Workspace {
        let mut workspace = Workspace {
            root: normalize(&root),
            manifest_override,
            manifest_path: None,
            manifest: None,
            manifest_errors: Vec::new(),
            manifest_text: String::new(),
            files: BTreeMap::new(),
            open: Vec::new(),
        };
        workspace.reload_manifest();
        workspace.rescan();
        workspace
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest_path(&self) -> Option<&Path> {
        self.manifest_path.as_deref()
    }

    pub fn manifest_text(&self) -> &str {
        &self.manifest_text
    }

    pub fn manifest_errors(&self) -> &[ManifestError] {
        &self.manifest_errors
    }

    pub fn registry(&self) -> Option<&fe_project::SymbolRegistry> {
        self.manifest.as_ref().map(|m| &m.registry)
    }

    pub fn mode(&self) -> Mode {
        if self.manifest.is_some() {
            return Mode::Semantic;
        }
        let reason = match &self.manifest_path {
            Some(path) => format!(
                "{} could not be read: {}",
                path.display(),
                self.manifest_errors
                    .first()
                    .map(|e| e.message.clone())
                    .unwrap_or_else(|| "unknown error".to_string())
            ),
            None => format!(
                "no {MANIFEST_NAME} in {} — only syntax is checked until one exists",
                self.root.display()
            ),
        };
        Mode::SyntaxOnly { reason }
    }

    pub fn document(&self, path: &Path) -> Option<&Document> {
        self.files.get(path)
    }

    /// Files in the order that assigns their `UnitId`s.
    pub fn iter(&self) -> impl Iterator<Item = (&PathBuf, &Document)> {
        self.files.iter()
    }

    pub fn is_open(&self, path: &Path) -> bool {
        self.open.iter().any(|p| p == path)
    }

    // ------------------------------------------------------------ mutation

    pub fn open(&mut self, path: PathBuf, text: String, version: i32) {
        let path = normalize(&path);
        if !self.open.contains(&path) {
            self.open.push(path.clone());
        }
        self.files.insert(path, Document::new(text, Some(version)));
    }

    pub fn change(&mut self, path: &Path, text: String, version: i32) {
        let path = &normalize(path);
        if let Some(document) = self.files.get_mut(path) {
            *document = Document::new(text, Some(version));
        } else {
            self.files
                .insert(path.to_path_buf(), Document::new(text, Some(version)));
        }
    }

    /// The buffer is gone; fall back to whatever is on disk.
    pub fn close(&mut self, path: &Path) {
        let path = &normalize(path);
        self.open.retain(|p| p != path);
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.files
                    .insert(path.to_path_buf(), Document::new(text, None));
            }
            Err(_) => {
                self.files.remove(path);
            }
        }
    }

    /// A file changed underneath us — a git checkout, a generator, another
    /// editor. An open buffer still wins: the client is the authority on a file
    /// it has open, and clobbering it would discard unsaved work.
    pub fn touch_on_disk(&mut self, path: &Path) {
        let path = &normalize(path);
        if self.is_open(path) {
            return;
        }
        if !is_source(path) {
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.files
                    .insert(path.to_path_buf(), Document::new(text, None));
            }
            Err(_) => {
                self.files.remove(path);
            }
        }
    }

    pub fn set_manifest_override(&mut self, path: Option<PathBuf>) -> bool {
        if self.manifest_override == path {
            return false;
        }
        self.manifest_override = path;
        self.reload_manifest();
        self.rescan();
        true
    }

    /// Re-read the manifest and re-walk the source roots. Called at startup and
    /// whenever `fe.toml` changes — its `sources` decide which files exist at
    /// all, so a change there can add or remove whole directories.
    pub fn reload(&mut self) {
        self.reload_manifest();
        self.rescan();
    }

    fn reload_manifest(&mut self) {
        self.manifest = None;
        self.manifest_errors.clear();
        self.manifest_text.clear();
        self.manifest_path = self
            .manifest_override
            .as_ref()
            .map(|path| normalize(&self.root.join(path)))
            .filter(|path| path.is_file())
            .or_else(|| find_manifest(&self.root));

        let Some(path) = self.manifest_path.clone() else {
            return;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                self.manifest_errors.push(ManifestError {
                    message: error.to_string(),
                    span: None,
                });
                return;
            }
        };
        match fe_project::parse(&text) {
            Ok(manifest) => self.manifest = Some(manifest),
            Err(errors) => self.manifest_errors = errors,
        }
        self.manifest_text = text;
    }

    /// Walk the manifest's source roots, keeping open buffers as they are.
    fn rescan(&mut self) {
        let base = self
            .manifest_path
            .as_ref()
            .and_then(|p| p.parent())
            .unwrap_or(&self.root)
            .to_path_buf();

        let sources = match &self.manifest {
            Some(manifest) => manifest.sources.clone(),
            // Without a manifest there is nothing to say which directories are
            // source roots, so the whole workspace is fair game.
            None => vec![fe_project::DEFAULT_SOURCE.to_string()],
        };

        let mut found = Vec::new();
        for source in &sources {
            let path = normalize(&base.join(source));
            if path.is_file() {
                found.push(path);
            } else {
                collect(&path, &mut found);
            }
        }

        let open: Vec<PathBuf> = self.open.clone();
        let previous = std::mem::take(&mut self.files);
        for path in found {
            if let Some(document) = previous.get(&path).filter(|_| open.contains(&path)) {
                self.files.insert(path, document.clone());
            } else if let Ok(text) = std::fs::read_to_string(&path) {
                self.files.insert(path, Document::new(text, None));
            }
        }
        // An open buffer outside every source root is still worth analysing —
        // otherwise opening a stray file gives no diagnostics and no clue why.
        for path in open {
            if !self.files.contains_key(&path) {
                if let Some(document) = previous.get(&path) {
                    self.files.insert(path, document.clone());
                }
            }
        }
    }
}

/// Remove `.` and resolve `..` without touching the filesystem.
///
/// `sources = ["."]` is the default, and joining it produces `<root>/./a.fe` —
/// a path that is the same file as `<root>/a.fe` and not the same `PathBuf`.
/// Every lookup here is by path equality, so the two would be different files:
/// an open buffer would not override what is on disk, and a published URI would
/// not be one the client recognises.
///
/// Lexical rather than [`std::fs::canonicalize`] on purpose. Canonicalising
/// resolves symlinks, and a client that opened the project through one would
/// then be sent URIs under a path it has never heard of.
pub fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // Only when there is something real to pop: `../a` at the start
                // of a relative path has to stay as it is.
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else {
                    out.push(component);
                }
            }
            other => out.push(other),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

pub fn is_source(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "fe")
}

pub fn is_manifest(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == MANIFEST_NAME)
}

/// The nearest `fe.toml` at or above `root`.
///
/// Searching upwards matters because an editor is often opened on a
/// subdirectory — `procedures/` rather than the addon root — and the manifest
/// belongs with the aircraft, not with the procedures.
fn find_manifest(root: &Path) -> Option<PathBuf> {
    let mut directory = Some(root);
    while let Some(current) = directory {
        let candidate = current.join(MANIFEST_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        directory = current.parent();
    }
    None
}

fn collect(directory: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if SKIP.contains(&name.as_ref()) || name.starts_with('.') {
                continue;
            }
            collect(&path, out);
        } else if is_source(&path) {
            out.push(path);
        }
    }
}
