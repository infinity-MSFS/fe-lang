"use strict";

// Just enough of the editor API — and of `vscode-languageclient` — for
// `extension.js` to run under plain node.
//
// It implements what the extension actually touches and nothing else, so it is
// a test of our wiring rather than of VS Code. If the extension starts using a
// new API, this file will throw rather than quietly pretend — which is the
// behaviour we want from it.

const Module = require("node:module");

class Disposable {
  constructor(onDispose) {
    this.onDispose = onDispose;
  }

  dispose() {
    if (this.onDispose) this.onDispose();
  }
}

class OutputChannel extends Disposable {
  constructor(name) {
    super();
    this.name = name;
    this.lines = [];
    this.shown = false;
  }

  appendLine(line) {
    this.lines.push(line);
  }

  show() {
    this.shown = true;
  }
}

class StatusBarItem extends Disposable {
  constructor() {
    super();
    this.text = "";
    this.tooltip = undefined;
    this.command = undefined;
    this.visible = false;
  }

  show() {
    this.visible = true;
  }

  hide() {
    this.visible = false;
  }
}

/** The state a test can inspect or arrange. */
const state = {
  settings: {},
  warnings: [],
  information: [],
  commands: new Map(),
  channels: [],
  statusBars: [],
  watchers: [],
};

function reset(settings = {}) {
  state.settings = settings;
  state.warnings = [];
  state.information = [];
  state.commands = new Map();
  state.channels = [];
  state.statusBars = [];
  state.watchers = [];
}

const vscode = {
  StatusBarAlignment: { Left: 1, Right: 2 },

  window: {
    createOutputChannel(name) {
      const channel = new OutputChannel(name);
      state.channels.push(channel);
      return channel;
    },
    createStatusBarItem() {
      const item = new StatusBarItem();
      state.statusBars.push(item);
      return item;
    },
    showWarningMessage(message) {
      state.warnings.push(message);
      return Promise.resolve(undefined);
    },
    showInformationMessage(message) {
      state.information.push(message);
      return Promise.resolve(undefined);
    },
  },

  workspace: {
    getConfiguration(section) {
      const values = state.settings[section] || {};
      return {
        get(key) {
          return values[key];
        },
      };
    },
    createFileSystemWatcher(pattern) {
      state.watchers.push(pattern);
      return new Disposable();
    },
  },

  commands: {
    registerCommand(name, handler) {
      state.commands.set(name, handler);
      return new Disposable(() => state.commands.delete(name));
    },
  },

  Disposable,
};

/** The bits of `vscode-languageclient/node` the extension uses. */
class LanguageClient {
  constructor(id, name, serverOptions, clientOptions) {
    this.id = id;
    this.name = name;
    this.serverOptions = serverOptions;
    this.clientOptions = clientOptions;
    this.started = false;
    this.restarts = 0;
    this.handlers = new Map();
  }

  onNotification(method, handler) {
    this.handlers.set(method, handler);
    return new Disposable();
  }

  /** Deliver a notification as the server would. */
  emit(method, params) {
    const handler = this.handlers.get(method);
    if (!handler) throw new Error(`nothing is listening for ${method}`);
    handler(params);
  }

  async start() {
    this.started = true;
  }

  async restart() {
    this.restarts += 1;
  }

  async stop() {
    this.started = false;
  }

  dispose() {}
}

const languageclient = {
  LanguageClient,
  TransportKind: { stdio: 0, ipc: 1, pipe: 2, socket: 3 },
};

// Make `require('vscode')` and `require('vscode-languageclient/node')` resolve
// to these, the way they do inside the editor.
const load = Module._load;
Module._load = function (request, parent, isMain) {
  if (request === "vscode") return vscode;
  if (request === "vscode-languageclient/node") return languageclient;
  return load.apply(this, [request, parent, isMain]);
};

module.exports = { vscode, languageclient, state, reset };
