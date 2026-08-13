'use strict';

// Just enough of the editor API for `extension.js` to run under plain node.
//
// It implements what the extension actually touches and nothing else, so it is
// a test of our wiring rather than of VS Code. If the extension starts using a
// new API, this file will throw rather than quietly pretend — which is the
// behaviour we want from it.

const fs = require('node:fs');
const path = require('node:path');
const Module = require('node:module');

class Position {
  constructor(line, character) {
    this.line = line;
    this.character = character;
  }
}

class Range {
  constructor(start, end) {
    this.start = start;
    this.end = end;
  }
}

class CompletionItem {
  constructor(label, kind) {
    this.label = label;
    this.kind = kind;
  }
}

class SnippetString {
  constructor(value) {
    this.value = value;
  }
}

class MarkdownString {
  constructor(value) {
    this.value = value;
  }
}

class DocumentSymbol {
  constructor(name, detail, kind, range, selectionRange) {
    Object.assign(this, { name, detail, kind, range, selectionRange });
  }
}

class Location {
  constructor(uri, position) {
    this.uri = uri;
    this.position = position;
  }
}

const DISPOSABLE = { dispose() {} };
const noEvent = () => DISPOSABLE;

/**
 * Build the stub. `files` is the workspace: a map of path to contents.
 */
function makeVscode(files) {
  const registered = {};
  const settings = { 'completion.enabled': true, 'completion.scanWorkspace': true };

  const uri = value => ({ toString: () => value, fsPath: value });

  const vscode = {
    Position,
    Range,
    CompletionItem,
    SnippetString,
    MarkdownString,
    DocumentSymbol,
    Location,
    Uri: { parse: uri },
    CompletionItemKind: {
      Keyword: 13,
      Constant: 20,
      Property: 9,
      Value: 11,
      Function: 2,
      Unit: 10,
    },
    SymbolKind: { Function: 11 },
    window: {
      createOutputChannel: () => ({ appendLine() {}, dispose() {} }),
    },
    workspace: {
      textDocuments: [],
      getConfiguration: () => ({ get: key => settings[key] }),
      findFiles: async () => Object.keys(files).map(uri),
      fs: { readFile: async target => Buffer.from(files[target.fsPath], 'utf8') },
      createFileSystemWatcher: () => ({
        onDidDelete: noEvent,
        onDidCreate: noEvent,
        dispose() {},
      }),
      onDidOpenTextDocument: noEvent,
      onDidSaveTextDocument: noEvent,
      onDidChangeConfiguration: noEvent,
      openTextDocument: async target => makeDocument(files[target.fsPath], target.fsPath),
    },
    languages: {
      registerCompletionItemProvider: (_selector, provider) => {
        registered.completion = provider;
        return DISPOSABLE;
      },
      registerDocumentSymbolProvider: (_selector, provider) => {
        registered.symbols = provider;
        return DISPOSABLE;
      },
      registerDefinitionProvider: (_selector, provider) => {
        registered.definition = provider;
        return DISPOSABLE;
      },
    },
  };

  function makeDocument(text, name = 'untitled.fe') {
    const offsetOf = position => {
      const lines = text.split('\n');
      let offset = 0;
      for (let i = 0; i < position.line; i += 1) offset += lines[i].length + 1;
      return offset + position.character;
    };

    return {
      uri: uri(name),
      languageId: 'fe',
      getText: range => (range ? text.slice(offsetOf(range.start), offsetOf(range.end)) : text),
      positionAt: offset => {
        const upto = text.slice(0, offset).split('\n');
        return new Position(upto.length - 1, upto[upto.length - 1].length);
      },
      getWordRangeAtPosition: (position, pattern) => {
        const line = text.split('\n')[position.line];
        for (const match of line.matchAll(new RegExp(pattern.source, 'g'))) {
          const start = match.index;
          const end = start + match[0].length;
          if (start <= position.character && position.character <= end) {
            return new Range(new Position(position.line, start), new Position(position.line, end));
          }
        }
        return undefined;
      },
      /** The position at the very end of the text — where the cursor is typing. */
      endPosition: () => {
        const lines = text.split('\n');
        return new Position(lines.length - 1, lines[lines.length - 1].length);
      },
    };
  }

  return { vscode, registered, makeDocument };
}

/** Load the extension with `require('vscode')` answered by the stub. */
function loadExtension(vscode) {
  const entry = path.join(__dirname, '..', 'src', 'extension.js');
  const load = Module._load;
  Module._load = function (request, ...rest) {
    if (request === 'vscode') return vscode;
    return load.call(this, request, ...rest);
  };
  try {
    delete require.cache[require.resolve(entry)];
    return require(entry);
  } finally {
    Module._load = load;
  }
}

/** Read a directory of .fe files into the shape `makeVscode` wants. */
function readWorkspace(directory) {
  const files = {};
  for (const name of fs.readdirSync(directory).filter(f => f.endsWith('.fe'))) {
    files[path.join(directory, name)] = fs.readFileSync(path.join(directory, name), 'utf8');
  }
  return files;
}

module.exports = { makeVscode, loadExtension, readWorkspace };
