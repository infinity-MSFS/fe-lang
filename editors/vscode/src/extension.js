"use strict";

const fs = require("fs");
const os = require("os");
const path = require("path");

const vscode = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

// The extension is a launcher and nothing more. Everything an editor asks about
// a .fe file — what is wrong with it, what could go here, what this name means —
// is answered by `fe-lsp`, against the aircraft's own `fe.toml`. Answering any
// of it here would mean a second implementation of the language's rules, and the
// second one is always the one that is wrong.

const BINARY = process.platform === "win32" ? "fe-lsp.exe" : "fe-lsp";

const INSTALL =
  "Install it with `cargo install --path fe-lsp` from a checkout of " +
  "https://github.com/infinity-MSFS/fe-lang, or set `fe.server.path`.";

/**
 * Where the server is.
 *
 * An explicit setting first, then PATH. Both are reported honestly: an
 * extension that silently did less because it could not find its server is
 * worse than one that says so, because a file with no errors shown looks
 * exactly like a file with no errors.
 */
function findServer(configuredPath) {
  if (configuredPath) {
    const resolved = configuredPath.replace(/^~(?=$|[/\\])/, os.homedir());
    if (!fs.existsSync(resolved)) {
      return {
        error: `\`fe.server.path\` is set to \`${resolved}\`, which does not exist.`,
      };
    }
    return { command: resolved };
  }

  const found = onPath(BINARY);
  if (found) return { command: found };
  return { error: `Could not find \`${BINARY}\` on your PATH. ${INSTALL}` };
}

function onPath(name) {
  const entries = (process.env.PATH || "")
    .split(path.delimiter)
    .filter(Boolean);
  for (const entry of entries) {
    const candidate = path.join(entry, name);
    try {
      fs.accessSync(candidate, fs.constants.X_OK);
      return candidate;
    } catch {
      // Not here, or not executable. Keep looking.
    }
  }
  return undefined;
}

/** The settings the server reads, in the shape it reads them. */
function initializationOptions() {
  const settings = vscode.workspace.getConfiguration("fe");
  return {
    manifest: settings.get("manifest") || "",
    inlayHints: { enable: settings.get("inlayHints.enable") },
    semanticTokens: { enable: settings.get("semanticTokens.enable") },
  };
}

async function activate(context) {
  const output = vscode.window.createOutputChannel("Flight Engineer");
  context.subscriptions.push(output);

  const configured = vscode.workspace.getConfiguration("fe").get("server.path");
  const server = findServer(configured);

  if (server.error) {
    output.appendLine(server.error);
    vscode.window.showWarningMessage(`Flight Engineer: ${server.error}`);
    return { client: undefined };
  }

  const run = { command: server.command, transport: TransportKind.stdio };
  const client = new LanguageClient(
    "fe",
    "Flight Engineer",
    { run, debug: run },
    {
      documentSelector: [{ scheme: "file", language: "fe" }],
      // The server watches these itself; VS Code does the watching and tells it.
      synchronize: {
        fileEvents: [
          vscode.workspace.createFileSystemWatcher("**/*.fe"),
          vscode.workspace.createFileSystemWatcher("**/fe.toml"),
        ],
      },
      initializationOptions: initializationOptions(),
      outputChannel: output,
    },
  );

  // Whether the aircraft's symbols were found decides how much of the language
  // is being checked, so it belongs in the status bar rather than in a popup
  // that is dismissed and forgotten.
  const status = vscode.window.createStatusBarItem(
    vscode.StatusBarAlignment.Right,
    0,
  );
  context.subscriptions.push(status);
  client.onNotification("fe/status", (params) => {
    status.text = params.semantic
      ? "$(check) fe"
      : "$(warning) fe: syntax only";
    status.tooltip = params.message;
    status.command = params.semantic ? undefined : "fe.showOutput";
    status.show();
  });

  context.subscriptions.push(
    vscode.commands.registerCommand("fe.showOutput", () => output.show()),
    vscode.commands.registerCommand("fe.restartServer", async () => {
      await client.restart();
    }),
  );

  await client.start();
  context.subscriptions.push(client);
  return { client };
}

function deactivate() {}

module.exports = { activate, deactivate, findServer, initializationOptions };
