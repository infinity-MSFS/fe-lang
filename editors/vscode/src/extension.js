'use strict';

const vscode = require('vscode');

const {
  CATEGORIES,
  COMMON_DURATIONS,
  COMMON_POSITIONS,
  contextAt,
  indexSource,
  loadSnippets,
  preview,
} = require('./analysis');

const SELECTOR = { language: 'fe' };
const EXCLUDE = '**/{node_modules,target,.git}/**';

// ---------------------------------------------------------------- the index

/**
 * What every .fe file in the workspace mentions. It is rebuilt per file on
 * save, on open and on change, and the file being edited is always re-read
 * live, so a control you have just typed is offered on the next line.
 */
class Index {
  constructor() {
    this.byFile = new Map();
  }

  update(uri, source) {
    this.byFile.set(uri.toString(), indexSource(source));
  }

  remove(uri) {
    this.byFile.delete(uri.toString());
  }

  clear() {
    this.byFile.clear();
  }

  /** Everything known, with `document` re-read so it is never stale. */
  merged(document) {
    const procedures = new Map();
    const controls = new Set();
    const positions = new Set();
    const paths = new Set();

    const files = new Map(this.byFile);
    if (document) files.set(document.uri.toString(), indexSource(document.getText()));

    for (const [uri, entry] of files) {
      for (const procedure of entry.procedures) {
        if (!procedures.has(procedure.name)) procedures.set(procedure.name, { ...procedure, uri });
      }
      for (const control of entry.controls) controls.add(control);
      for (const position of entry.positions) positions.add(position);
      for (const p of entry.paths) paths.add(p);
    }

    return {
      procedures: [...procedures.values()],
      controls: [...controls].sort(),
      positions: [...positions].sort(),
      paths: [...paths].sort(),
    };
  }

  /** Where a procedure of this name is declared, if anywhere. */
  find(name, document) {
    return this.merged(document).procedures.find(procedure => procedure.name === name);
  }
}

// ------------------------------------------------------------- completions

// `sortText` decides the order of the list. Names are left to sort themselves
// alphabetically; keywords and durations are given their position explicitly, so
// that a procedure is offered `name` before `revision` and 500ms before 5m,
// which is the order they are written in rather than the order they spell.
function sortKey(group, order) {
  return order === undefined ? String(group) : `${group}${String(order).padStart(3, '0')}`;
}

function snippetItem(snippet, group, order) {
  const item = new vscode.CompletionItem(snippet.prefix, vscode.CompletionItemKind.Keyword);
  item.insertText = new vscode.SnippetString(snippet.body);
  item.detail = snippet.description;
  item.documentation = new vscode.MarkdownString(
    ['```fe', preview(snippet.body), '```'].join('\n'),
  );
  item.sortText = sortKey(group, order);
  return item;
}

function valueItem(label, kind, detail, group, order) {
  const item = new vscode.CompletionItem(label, kind);
  if (detail) item.detail = detail;
  item.sortText = order === undefined ? `${group}${label}` : sortKey(group, order);
  return item;
}

function completionsFor(context, snippets, known) {
  const { Constant, Property, Value, Function: Fn, Unit, Keyword } = vscode.CompletionItemKind;
  const inContext = where => snippets.filter(snippet => snippet.context === where);

  switch (context.kind) {
    case 'none':
      return [];

    case 'top':
      return inContext('top').map((s, i) => snippetItem(s, '0', i));

    case 'metadata':
      return [
        ...inContext('metadata').map((s, i) => snippetItem(s, '0', i)),
        ...inContext('step').map((s, i) => snippetItem(s, '1', i)),
      ];

    case 'step':
      return inContext('step').map((s, i) => snippetItem(s, '0', i));

    case 'category':
      return CATEGORIES.map((c, i) => valueItem(c, Value, 'category', '0', i));

    case 'duration':
      return COMMON_DURATIONS.map((d, i) => valueItem(d, Unit, 'duration — ms, s or m', '0', i));

    case 'position': {
      const used = known.positions;
      return [
        ...used.map(p => valueItem(p, Constant, 'used in this workspace', '0')),
        ...COMMON_POSITIONS.filter(p => !used.includes(p)).map(p => valueItem(p, Constant, '', '1')),
      ];
    }

    case 'control':
      return known.controls.map(c => valueItem(c, Constant, 'control', '0'));

    case 'procedure':
      return known.procedures.map(p => valueItem(p.name, Fn, p.title, '0'));

    case 'expression':
      return [
        ...known.paths.map(p => valueItem(p, Property, 'aircraft state', '0')),
        valueItem('true', Value, '', '1'),
        valueItem('false', Value, '', '1'),
        valueItem('timeout', Keyword, 'timeout DURATION [else fail]', '2'),
      ];

    default:
      return [];
  }
}

// ------------------------------------------------------------------ wiring

function activate(context) {
  const index = new Index();
  const snippets = loadSnippets(context.extensionPath);
  const output = vscode.window.createOutputChannel('Flight Engineer');
  context.subscriptions.push(output);

  const setting = key => vscode.workspace.getConfiguration('fe').get(key);

  const indexDocument = document => {
    if (document.languageId === 'fe') index.update(document.uri, document.getText());
  };

  const indexWorkspace = async () => {
    index.clear();
    if (!setting('completion.scanWorkspace')) return;
    const files = await vscode.workspace.findFiles('**/*.fe', EXCLUDE);
    for (const uri of files) {
      try {
        const bytes = await vscode.workspace.fs.readFile(uri);
        index.update(uri, Buffer.from(bytes).toString('utf8'));
      } catch (error) {
        output.appendLine(`could not read ${uri.fsPath}: ${error}`);
      }
    }
  };

  const ready = indexWorkspace();

  const watcher = vscode.workspace.createFileSystemWatcher('**/*.fe');
  context.subscriptions.push(
    watcher,
    watcher.onDidDelete(uri => index.remove(uri)),
    watcher.onDidCreate(async uri => {
      if (!setting('completion.scanWorkspace')) return;
      const bytes = await vscode.workspace.fs.readFile(uri);
      index.update(uri, Buffer.from(bytes).toString('utf8'));
    }),
    vscode.workspace.onDidOpenTextDocument(indexDocument),
    vscode.workspace.onDidSaveTextDocument(indexDocument),
    vscode.workspace.onDidChangeConfiguration(event => {
      if (event.affectsConfiguration('fe.completion.scanWorkspace')) indexWorkspace();
    }),
  );
  vscode.workspace.textDocuments.forEach(indexDocument);

  context.subscriptions.push(
    vscode.languages.registerCompletionItemProvider(
      SELECTOR,
      {
        provideCompletionItems(document, position) {
          if (!setting('completion.enabled')) return [];
          const before = document.getText(new vscode.Range(new vscode.Position(0, 0), position));
          const context = contextAt(before);
          if (context.kind === 'none') return [];
          return completionsFor(context, snippets, index.merged(document));
        },
      },
      '.',
      ' ',
      '=',
    ),

    vscode.languages.registerDocumentSymbolProvider(SELECTOR, {
      provideDocumentSymbols(document) {
        const { procedures } = indexSource(document.getText());
        return procedures.map(procedure => {
          const range = new vscode.Range(
            document.positionAt(procedure.index),
            document.positionAt(procedure.end),
          );
          const selection = new vscode.Range(
            document.positionAt(procedure.index),
            document.positionAt(procedure.index + 'procedure '.length + procedure.name.length),
          );
          return new vscode.DocumentSymbol(
            procedure.name,
            procedure.title,
            vscode.SymbolKind.Function,
            range,
            selection,
          );
        });
      },
    }),

    // `call HYD_2_ELECTRIC_PUMP_START` jumps to the procedure, wherever in the
    // workspace it is declared — procedure identifiers share one flat namespace
    // across every file compiled together.
    vscode.languages.registerDefinitionProvider(SELECTOR, {
      async provideDefinition(document, position) {
        const range = document.getWordRangeAtPosition(position, /[A-Za-z_][A-Za-z0-9_]*/);
        if (!range) return undefined;
        const found = index.find(document.getText(range), document);
        if (!found) return undefined;
        const target = await vscode.workspace.openTextDocument(vscode.Uri.parse(found.uri));
        return new vscode.Location(target.uri, target.positionAt(found.index));
      },
    }),
  );

  // `ready` resolves when the first sweep of the workspace has finished. VS Code
  // does not wait for it — completion works from the open file immediately — but
  // the tests need to know when the index is populated.
  return { ready };
}

function deactivate() {}

module.exports = { activate, deactivate };
