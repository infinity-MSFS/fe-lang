//! The message loop and the request handlers.

use std::collections::HashMap;
use std::path::PathBuf;

use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use lsp_types::*;

use fe_lang::span::UnitId;

use crate::analysis::Analysis;
use crate::completion::{Context, context_at};
use crate::config::Config;
use crate::features::{actions, format, hover, items, tokens};
use crate::line_index::{Encoding, LineIndex};
use crate::locate::{self, Role};
use crate::uri;
use crate::workspace::{self, Mode, Workspace};

pub struct Server {
    connection: Connection,
    workspace: Workspace,
    config: Config,
    encoding: Encoding,
    snippets: Vec<items::Snippet>,
    analysis: Option<Analysis>,
    /// Sources changed since the last analysis.
    stale: bool,
    /// Diagnostics should be republished once the current burst settles.
    needs_publish: bool,
    /// What was last published where, so a file whose problems are gone gets an
    /// empty list rather than keeping a squiggle for a fixed error.
    published: HashMap<PathBuf, usize>,
    /// The last mode reported to the client, so the explanation is sent once
    /// rather than on every keystroke.
    reported_mode: Option<Mode>,
}

pub fn capabilities(encoding: Encoding) -> ServerCapabilities {
    ServerCapabilities {
        position_encoding: Some(encoding.kind()),
        // Full sync. A procedure file is a few kilobytes; incremental sync
        // would buy nothing and could desynchronise.
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![".".into(), " ".into(), "=".into()]),
            ..Default::default()
        }),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        workspace_symbol_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Right(RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: Default::default(),
        })),
        code_action_provider: Some(CodeActionProviderCapability::Options(CodeActionOptions {
            code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
            ..Default::default()
        })),
        document_formatting_provider: Some(OneOf::Left(true)),
        inlay_hint_provider: Some(OneOf::Left(true)),
        semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
            SemanticTokensOptions {
                legend: SemanticTokensLegend {
                    token_types: tokens::LEGEND.to_vec(),
                    token_modifiers: Vec::new(),
                },
                full: Some(SemanticTokensFullOptions::Bool(true)),
                ..Default::default()
            },
        )),
        ..Default::default()
    }
}

impl Server {
    pub fn new(connection: Connection, params: InitializeParams) -> Server {
        let encoding = Encoding::negotiate(
            params
                .capabilities
                .general
                .as_ref()
                .and_then(|general| general.position_encodings.as_deref()),
        );
        let config = Config::from_initialization(params.initialization_options.as_ref());
        let root = root_of(&params).unwrap_or_else(|| PathBuf::from("."));
        let workspace = Workspace::new(root, config.manifest.clone());

        Server {
            connection,
            workspace,
            config,
            encoding,
            snippets: items::snippets(),
            analysis: None,
            stale: true,
            needs_publish: true,
            published: HashMap::new(),
            reported_mode: None,
        }
    }

    pub fn run(mut self) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
        self.register_watchers();
        self.publish();

        while let Ok(message) = self.connection.receiver.recv() {
            if self.dispatch(message)? {
                return Ok(());
            }
            // Drain whatever else is already queued before recomputing. A burst
            // of keystrokes then costs one analysis rather than one each, with
            // no timer and no background thread to get out of step with.
            while let Ok(message) = self.connection.receiver.try_recv() {
                if self.dispatch(message)? {
                    return Ok(());
                }
            }
            if self.needs_publish {
                self.publish();
            }
        }
        Ok(())
    }

    /// Returns `true` when the client has asked the server to stop.
    fn dispatch(
        &mut self,
        message: Message,
    ) -> Result<bool, Box<dyn std::error::Error + Sync + Send>> {
        match message {
            Message::Request(request) => {
                if self.connection.handle_shutdown(&request)? {
                    return Ok(true);
                }
                // Answering a question requires the analysis anyway, so publish
                // first when it is pending. It costs one message and it keeps
                // the two halves of what the client is told — the squiggles and
                // the answer it just asked for — describing the same state.
                if self.needs_publish {
                    self.publish();
                }
                let id = request.id.clone();
                let response = self.request(request);
                self.connection
                    .sender
                    .send(Message::Response(match response {
                        Ok(value) => Response::new_ok(id, value),
                        Err(message) => {
                            Response::new_err(id, ErrorCode::InternalError as i32, message)
                        }
                    }))?;
            }
            Message::Notification(notification) => self.notification(notification),
            Message::Response(_) => {}
        }
        Ok(false)
    }

    // ------------------------------------------------------------- analysis

    fn analysis(&mut self) -> &Analysis {
        if self.stale || self.analysis.is_none() {
            self.analysis = Some(Analysis::run(&self.workspace));
            self.stale = false;
        }
        self.analysis.as_ref().expect("just computed")
    }

    /// Publish diagnostics for every file, including the ones with none.
    fn publish(&mut self) {
        self.needs_publish = false;
        let encoding = self.encoding;
        let batches = {
            let analysis = self.analysis();
            collect(analysis, encoding)
        };

        let mut current = HashMap::new();
        for (path, diagnostics) in batches {
            current.insert(path.clone(), diagnostics.len());
            self.send_diagnostics(path, diagnostics);
        }

        // Anything reported on before that is not a source any more — deleted,
        // or dropped out of the manifest's `sources`. Without this an error
        // stays on screen after the file it was in has gone.
        let gone: Vec<PathBuf> = self
            .published
            .keys()
            .filter(|path| !current.contains_key(*path))
            .cloned()
            .collect();
        for path in gone {
            self.send_diagnostics(path, Vec::new());
        }
        self.published = current;

        self.report_manifest();
        self.report_mode();
    }

    fn send_diagnostics(&self, path: PathBuf, diagnostics: Vec<Diagnostic>) {
        let Some(uri) = uri::from_path(&path) else {
            return;
        };
        self.notify::<lsp_types::notification::PublishDiagnostics>(PublishDiagnosticsParams {
            uri,
            diagnostics,
            version: None,
        });
    }

    /// A broken `fe.toml` is reported against `fe.toml`, where it can be fixed.
    fn report_manifest(&self) {
        let Some(path) = self.workspace.manifest_path() else {
            return;
        };
        let text = self.workspace.manifest_text();
        let index = LineIndex::new(text);
        let diagnostics = self
            .workspace
            .manifest_errors()
            .iter()
            .map(|error| {
                let span = error.span.clone().unwrap_or(0..0);
                Diagnostic {
                    range: Range {
                        start: index.position(text, span.start as u32, self.encoding),
                        end: index.position(text, span.end as u32, self.encoding),
                    },
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("fe".to_string()),
                    message: error.message.clone(),
                    ..Default::default()
                }
            })
            .collect();
        self.send_diagnostics(path.to_path_buf(), diagnostics);
    }

    /// Say once, and only when it changes, how much the server can check.
    ///
    /// Silence would be worse than the limitation: an author whose file shows no
    /// errors is entitled to assume it has none.
    fn report_mode(&mut self) {
        let mode = self.workspace.mode();
        if self.reported_mode.as_ref() == Some(&mode) {
            return;
        }
        self.reported_mode = Some(mode.clone());

        let (typ, message) = match &mode {
            Mode::Semantic => (
                MessageType::INFO,
                format!(
                    "fe: checking against {}",
                    self.workspace
                        .manifest_path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                ),
            ),
            Mode::SyntaxOnly { reason } => {
                (MessageType::WARNING, format!("fe: syntax-only — {reason}"))
            }
        };
        self.notify::<lsp_types::notification::ShowMessage>(ShowMessageParams {
            typ,
            message: message.clone(),
        });
        // Also as data, so a client can render it in a status bar rather than a
        // popup.
        self.send(Message::Notification(Notification::new(
            "fe/status".to_string(),
            serde_json::json!({
                "semantic": mode.is_semantic(),
                "manifest": self.workspace.manifest_path().map(|p| p.display().to_string()),
                "message": message,
            }),
        )));
    }

    fn register_watchers(&self) {
        let watchers = ["**/*.fe", "**/fe.toml"]
            .into_iter()
            .map(|pattern| FileSystemWatcher {
                glob_pattern: GlobPattern::String(pattern.to_string()),
                kind: None,
            })
            .collect();
        self.send(Message::Request(Request::new(
            RequestId::from("fe-watch-files".to_string()),
            lsp_types::request::RegisterCapability::METHOD.to_string(),
            RegistrationParams {
                registrations: vec![Registration {
                    id: "fe-watch-files".to_string(),
                    method: lsp_types::notification::DidChangeWatchedFiles::METHOD.to_string(),
                    register_options: serde_json::to_value(
                        DidChangeWatchedFilesRegistrationOptions { watchers },
                    )
                    .ok(),
                }],
            },
        )));
    }

    fn notify<N: lsp_types::notification::Notification>(&self, params: N::Params) {
        self.send(Message::Notification(Notification::new(
            N::METHOD.to_string(),
            params,
        )));
    }

    fn send(&self, message: Message) {
        let _ = self.connection.sender.send(message);
    }

    // -------------------------------------------------------- notifications

    fn notification(&mut self, notification: Notification) {
        let method = notification.method.clone();
        match method.as_str() {
            lsp_types::notification::DidOpenTextDocument::METHOD => {
                if let Ok(params) = notification.extract::<DidOpenTextDocumentParams>(&method) {
                    if let Some(path) = uri::to_path(&params.text_document.uri) {
                        self.workspace.open(
                            path,
                            params.text_document.text,
                            params.text_document.version,
                        );
                        self.invalidate();
                    }
                }
            }
            lsp_types::notification::DidChangeTextDocument::METHOD => {
                if let Ok(params) = notification.extract::<DidChangeTextDocumentParams>(&method) {
                    // Full sync, so the last change carries the whole document.
                    if let (Some(path), Some(change)) = (
                        uri::to_path(&params.text_document.uri),
                        params.content_changes.into_iter().next_back(),
                    ) {
                        self.workspace
                            .change(&path, change.text, params.text_document.version);
                        self.invalidate();
                    }
                }
            }
            lsp_types::notification::DidCloseTextDocument::METHOD => {
                if let Ok(params) = notification.extract::<DidCloseTextDocumentParams>(&method) {
                    if let Some(path) = uri::to_path(&params.text_document.uri) {
                        self.workspace.close(&path);
                        self.invalidate();
                    }
                }
            }
            lsp_types::notification::DidChangeWatchedFiles::METHOD => {
                if let Ok(params) = notification.extract::<DidChangeWatchedFilesParams>(&method) {
                    let mut reload = false;
                    for event in params.changes {
                        let Some(path) = uri::to_path(&event.uri) else {
                            continue;
                        };
                        if workspace::is_manifest(&path) {
                            reload = true;
                        } else {
                            self.workspace.touch_on_disk(&path);
                        }
                    }
                    // A change to `sources` can add or remove whole directories,
                    // so the manifest is reloaded before the files are.
                    if reload {
                        self.workspace.reload();
                    }
                    self.invalidate();
                }
            }
            lsp_types::notification::DidChangeConfiguration::METHOD => {
                if let Ok(params) = notification.extract::<DidChangeConfigurationParams>(&method) {
                    self.config = Config::from_settings(&params.settings);
                    if self
                        .workspace
                        .set_manifest_override(self.config.manifest.clone())
                    {
                        self.invalidate();
                    }
                }
            }
            _ => {}
        }
    }

    fn invalidate(&mut self) {
        self.stale = true;
        self.needs_publish = true;
    }

    // ------------------------------------------------------------- requests

    fn request(&mut self, request: Request) -> Result<serde_json::Value, String> {
        let method = request.method.clone();
        macro_rules! handle {
            ($request:ty, $handler:ident) => {
                if method == <$request as lsp_types::request::Request>::METHOD {
                    let (_, params) = request
                        .extract::<<$request as lsp_types::request::Request>::Params>(&method)
                        .map_err(|error| error.to_string())?;
                    let result = self.$handler(params);
                    return serde_json::to_value(result).map_err(|error| error.to_string());
                }
            };
        }

        handle!(lsp_types::request::Completion, completion);
        handle!(lsp_types::request::HoverRequest, hover);
        handle!(lsp_types::request::GotoDefinition, definition);
        handle!(lsp_types::request::References, references);
        handle!(lsp_types::request::DocumentSymbolRequest, document_symbols);
        handle!(
            lsp_types::request::WorkspaceSymbolRequest,
            workspace_symbols
        );
        handle!(lsp_types::request::PrepareRenameRequest, prepare_rename);
        handle!(lsp_types::request::Rename, rename);
        handle!(lsp_types::request::CodeActionRequest, code_actions);
        handle!(lsp_types::request::Formatting, formatting);
        handle!(lsp_types::request::InlayHintRequest, inlay_hints);
        handle!(
            lsp_types::request::SemanticTokensFullRequest,
            semantic_tokens
        );

        Ok(serde_json::Value::Null)
    }

    /// The document a request names, as a unit of the current analysis.
    fn resolve(&mut self, uri: &Uri) -> Option<(UnitId, String, LineIndex)> {
        let path = uri::to_path(uri)?;
        let analysis = self.analysis();
        let unit = analysis.unit_of(&path)?;
        let text = analysis.text(unit)?.to_string();
        let index = LineIndex::new(&text);
        Some((unit, text, index))
    }

    fn offset_at(&mut self, uri: &Uri, position: Position) -> Option<(UnitId, String, u32)> {
        let (unit, text, index) = self.resolve(uri)?;
        let offset = index.offset(&text, position, self.encoding);
        Some((unit, text, offset))
    }

    fn completion(&mut self, params: CompletionParams) -> Option<CompletionResponse> {
        let position = params.text_document_position.position;
        let (_, text, offset) =
            self.offset_at(&params.text_document_position.text_document.uri, position)?;
        let context = context_at(&text[..offset as usize]);
        if context == Context::None {
            return None;
        }
        // Refresh first, then borrow the field: `completions` needs the
        // analysis and the snippets at the same time.
        self.analysis();
        let analysis = self.analysis.as_ref().expect("just refreshed");
        let items = items::completions(&context, analysis, &self.snippets);
        Some(CompletionResponse::Array(items))
    }

    fn hover(&mut self, params: HoverParams) -> Option<Hover> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let (unit, text, offset) = self.offset_at(uri, position)?;
        let analysis = self.analysis();
        let occurrence = locate::at(unit, analysis.ast(unit)?, offset)?;
        let markdown = hover::markdown(analysis, &occurrence)?;
        let index = LineIndex::new(&text);
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: markdown,
            }),
            range: Some(index.range(&text, occurrence.span, self.encoding)),
        })
    }

    fn definition(&mut self, params: GotoDefinitionParams) -> Option<GotoDefinitionResponse> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let (unit, _, offset) = self.offset_at(uri, position)?;
        let encoding = self.encoding;
        let analysis = self.analysis();
        let occurrence = locate::at(unit, analysis.ast(unit)?, offset)?;
        if !occurrence.role.is_procedure() {
            return None;
        }
        // One flat namespace across every file compiled together, so the
        // declaration may be anywhere in the project.
        let (target_unit, decl) = analysis.procedure(&occurrence.text)?;
        let path = analysis.path(target_unit)?;
        let text = analysis.text(target_unit)?;
        Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri::from_path(path)?,
            range: LineIndex::new(text).range(text, decl.id.span, encoding),
        }))
    }

    fn references(&mut self, params: ReferenceParams) -> Option<Vec<Location>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let include_declaration = params.context.include_declaration;
        let (unit, _, offset) = self.offset_at(uri, position)?;
        let encoding = self.encoding;
        let analysis = self.analysis();
        let occurrence = locate::at(unit, analysis.ast(unit)?, offset)?;

        let wanted = occurrence.text.clone();
        let procedures = occurrence.role.is_procedure();

        let mut out = Vec::new();
        for (other, ast) in analysis.asts() {
            let Some(path) = analysis.path(other) else {
                continue;
            };
            let Some(text) = analysis.text(other) else {
                continue;
            };
            let Some(uri) = uri::from_path(path) else {
                continue;
            };
            let index = LineIndex::new(text);
            for found in locate::occurrences(other, ast) {
                if found.text != wanted {
                    continue;
                }
                // A control and a procedure could share a spelling; only report
                // the same kind of thing.
                let same_kind = if procedures {
                    found.role.is_procedure()
                } else {
                    matches!(found.role, Role::Control { .. } | Role::State)
                };
                if !same_kind {
                    continue;
                }
                if !include_declaration && found.role == Role::ProcedureDecl {
                    continue;
                }
                out.push(Location {
                    uri: uri.clone(),
                    range: index.range(text, found.span, encoding),
                });
            }
        }
        Some(out)
    }

    fn document_symbols(&mut self, params: DocumentSymbolParams) -> Option<DocumentSymbolResponse> {
        let encoding = self.encoding;
        let (unit, text, index) = self.resolve(&params.text_document.uri)?;
        let analysis = self.analysis();
        let ast = analysis.ast(unit)?;

        #[allow(deprecated)] // `deprecated` is a required field of the struct
        let symbols = ast
            .procedures
            .iter()
            .map(|decl| DocumentSymbol {
                name: decl.id.text.clone(),
                detail: decl.metadata.name.as_ref().map(|n| n.value.clone()),
                kind: SymbolKind::FUNCTION,
                tags: None,
                deprecated: None,
                range: index.range(&text, decl.span, encoding),
                selection_range: index.range(&text, decl.id.span, encoding),
                children: None,
            })
            .collect();
        Some(DocumentSymbolResponse::Nested(symbols))
    }

    fn workspace_symbols(
        &mut self,
        params: WorkspaceSymbolParams,
    ) -> Option<Vec<SymbolInformation>> {
        let query = params.query.to_lowercase();
        let encoding = self.encoding;
        let analysis = self.analysis();

        #[allow(deprecated)]
        let symbols = analysis
            .procedures()
            .filter(|(_, decl)| query.is_empty() || decl.id.text.to_lowercase().contains(&query))
            .filter_map(|(unit, decl)| {
                let path = analysis.path(unit)?;
                let text = analysis.text(unit)?;
                Some(SymbolInformation {
                    name: decl.id.text.clone(),
                    kind: SymbolKind::FUNCTION,
                    tags: None,
                    deprecated: None,
                    location: Location {
                        uri: uri::from_path(path)?,
                        range: LineIndex::new(text).range(text, decl.id.span, encoding),
                    },
                    container_name: decl.metadata.name.as_ref().map(|n| n.value.clone()),
                })
            })
            .collect();
        Some(symbols)
    }

    fn prepare_rename(
        &mut self,
        params: TextDocumentPositionParams,
    ) -> Option<PrepareRenameResponse> {
        let (unit, text, offset) = self.offset_at(&params.text_document.uri, params.position)?;
        let encoding = self.encoding;
        let analysis = self.analysis();
        let occurrence = locate::at(unit, analysis.ast(unit)?, offset)?;
        // Only procedure identifiers. A control's name belongs to the aircraft's
        // manifest and its systems code; renaming it here would rename half of
        // a relationship.
        if !occurrence.role.is_procedure() {
            return None;
        }
        let index = LineIndex::new(&text);
        Some(PrepareRenameResponse::Range(index.range(
            &text,
            occurrence.span,
            encoding,
        )))
    }

    fn rename(&mut self, params: RenameParams) -> Option<WorkspaceEdit> {
        let position = params.text_document_position;
        let (unit, _, offset) = self.offset_at(&position.text_document.uri, position.position)?;
        let encoding = self.encoding;
        let analysis = self.analysis();
        let occurrence = locate::at(unit, analysis.ast(unit)?, offset)?;
        if !occurrence.role.is_procedure() {
            return None;
        }

        let mut changes: HashMap<Uri, Vec<TextEdit>> = HashMap::new();
        for (other, ast) in analysis.asts() {
            let (Some(path), Some(text)) = (analysis.path(other), analysis.text(other)) else {
                continue;
            };
            let Some(uri) = uri::from_path(path) else {
                continue;
            };
            let index = LineIndex::new(text);
            for found in locate::occurrences(other, ast) {
                if found.text == occurrence.text && found.role.is_procedure() {
                    changes.entry(uri.clone()).or_default().push(TextEdit {
                        range: index.range(text, found.span, encoding),
                        new_text: params.new_name.clone(),
                    });
                }
            }
        }
        Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        })
    }

    fn code_actions(&mut self, params: CodeActionParams) -> Option<CodeActionResponse> {
        let uri = params.text_document.uri.clone();
        let (unit, text, index) = self.resolve(&uri)?;
        let encoding = self.encoding;
        let offset = index.offset(&text, params.range.start, encoding);
        let analysis = self.analysis();
        let occurrence = locate::at(unit, analysis.ast(unit)?, offset);
        let registry = analysis.registry.as_ref();

        let mut out: CodeActionResponse = Vec::new();
        for diagnostic in &params.context.diagnostics {
            let Some(NumberOrString::String(code)) = &diagnostic.code else {
                continue;
            };
            let range = diagnostic.range;
            let start = index.offset(&text, range.start, encoding) as usize;
            let end = index.offset(&text, range.end, encoding) as usize;
            let Some(covered) = text.get(start..end) else {
                continue;
            };

            // Most diagnostics point at the thing to change, and the text under
            // the span is what the fix is about. Two do not, and each needs the
            // walker to find what the span is *about* rather than what it is on.
            let fixes = match code.as_str() {
                // The span is the position; the control whose positions are
                // valid is the one the same statement names.
                fe_compiler::codes::INVALID_CONTROL_VALUE => {
                    match occurrence.as_ref().map(|o| o.role.clone()) {
                        Some(Role::Position { control }) => {
                            actions::position_fixes(registry, &control)
                        }
                        _ => Vec::new(),
                    }
                }
                // The span is the verb; the control is the next one written.
                fe_compiler::codes::INVALID_ACTION_FOR_CONTROL => {
                    match analysis
                        .ast(unit)
                        .and_then(|ast| locate::control_after(unit, ast, offset))
                    {
                        Some(control) => {
                            let verb = match control.role {
                                Role::Control { verb } => Some(verb),
                                _ => None,
                            };
                            actions::fixes(code, &control.text, registry, verb)
                        }
                        None => Vec::new(),
                    }
                }
                _ => actions::fixes(code, covered, registry, None),
            };

            for fix in fixes {
                out.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: fix.title,
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![diagnostic.clone()]),
                    is_preferred: fix.preferred.then_some(true),
                    edit: Some(WorkspaceEdit {
                        changes: Some(HashMap::from([(
                            uri.clone(),
                            vec![TextEdit {
                                range,
                                new_text: fix.replacement,
                            }],
                        )])),
                        ..Default::default()
                    }),
                    ..Default::default()
                }));
            }
        }
        Some(out)
    }

    fn formatting(&mut self, params: DocumentFormattingParams) -> Option<Vec<TextEdit>> {
        let (_, text, index) = self.resolve(&params.text_document.uri)?;
        let formatted = format::format(&text)?;
        Some(vec![TextEdit {
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: index.position(&text, text.len() as u32, self.encoding),
            },
            new_text: formatted,
        }])
    }

    fn inlay_hints(&mut self, params: InlayHintParams) -> Option<Vec<InlayHint>> {
        if !self.config.inlay_hints {
            return Some(Vec::new());
        }
        let encoding = self.encoding;
        let (unit, text, index) = self.resolve(&params.text_document.uri)?;
        let analysis = self.analysis();

        let hints = tokens::inlay_hints(analysis, unit)
            .into_iter()
            .map(|(span, label)| InlayHint {
                position: index.position(&text, span.end, encoding),
                label: InlayHintLabel::String(label),
                kind: Some(InlayHintKind::TYPE),
                text_edits: None,
                tooltip: None,
                padding_left: Some(true),
                padding_right: None,
                data: None,
            })
            .collect();
        Some(hints)
    }

    fn semantic_tokens(&mut self, params: SemanticTokensParams) -> Option<SemanticTokensResult> {
        if !self.config.semantic_tokens {
            return None;
        }
        let encoding = self.encoding;
        let (unit, _, index) = self.resolve(&params.text_document.uri)?;
        let analysis = self.analysis();
        Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: tokens::semantic_tokens(analysis, unit, &index, encoding),
        }))
    }
}

/// Diagnostics for every file the analysis covers, bucketed by the file they
/// belong to. Files with none get an empty list rather than being left out:
/// nothing else tells a client that a fixed error is fixed.
fn collect(analysis: &Analysis, encoding: Encoding) -> Vec<(PathBuf, Vec<Diagnostic>)> {
    let indexes = |unit: UnitId| analysis.text(unit).map(LineIndex::new);

    let mut by_unit: HashMap<usize, Vec<Diagnostic>> = HashMap::new();
    for diagnostic in &analysis.diagnostics {
        let unit = diagnostic.primary.span.unit;
        if let Some(converted) =
            crate::convert::diagnostic(analysis, &indexes, diagnostic, encoding)
        {
            by_unit.entry(unit.0 as usize).or_default().push(converted);
        }
    }

    analysis
        .paths
        .iter()
        .enumerate()
        .map(|(index, path)| (path.clone(), by_unit.remove(&index).unwrap_or_default()))
        .collect()
}

fn root_of(params: &InitializeParams) -> Option<PathBuf> {
    if let Some(folders) = &params.workspace_folders {
        if let Some(folder) = folders.first() {
            return uri::to_path(&folder.uri);
        }
    }
    #[allow(deprecated)]
    if let Some(uri) = &params.root_uri {
        return uri::to_path(uri);
    }
    std::env::current_dir().ok()
}
