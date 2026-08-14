"use strict";

// The extension is a launcher, so this tests launching: finding the server,
// passing the settings on, reporting what it could not do, and reflecting the
// server's status. Everything about the *language* is tested in `fe-lsp`, where
// it is implemented.

const assert = require("node:assert");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { test, beforeEach } = require("node:test");

const { state, reset } = require("./vscode-stub");
const extension = require("../src/extension");

const IS_WINDOWS = process.platform === "win32";
const BINARY = IS_WINDOWS ? "fe-lsp.exe" : "fe-lsp";

/** A directory containing an executable stand-in for the server. */
function withFakeServer(name = BINARY) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "fe-vscode-"));
  const binary = path.join(directory, name);
  fs.writeFileSync(binary, "#!/bin/sh\nexit 0\n");
  fs.chmodSync(binary, 0o755);
  return { directory, binary };
}

function context() {
  return { subscriptions: [], extensionPath: path.join(__dirname, "..") };
}

beforeEach(() => reset());

test("the server is found on PATH", () => {
  const { directory, binary } = withFakeServer();
  const previous = process.env.PATH;
  process.env.PATH = directory;
  try {
    assert.deepEqual(extension.findServer(undefined), { command: binary });
  } finally {
    process.env.PATH = previous;
  }
});

test("an explicit path wins over PATH", () => {
  const onPath = withFakeServer();
  const configured = withFakeServer("elsewhere");
  const previous = process.env.PATH;
  process.env.PATH = onPath.directory;
  try {
    const found = extension.findServer(configured.binary);
    assert.equal(found.command, configured.binary);
  } finally {
    process.env.PATH = previous;
  }
});

test("a configured path that does not exist is an error, not a fallback", () => {
  const { directory } = withFakeServer();
  const previous = process.env.PATH;
  process.env.PATH = directory;
  try {
    const found = extension.findServer(path.join(directory, "not-here"));
    assert.equal(found.command, undefined);
    assert.match(found.error, /does not exist/);
  } finally {
    process.env.PATH = previous;
  }
});

test("a missing server explains how to get one", () => {
  const previous = process.env.PATH;
  process.env.PATH = "";
  try {
    const found = extension.findServer(undefined);
    assert.equal(found.command, undefined);
    assert.match(found.error, /cargo install --path fe-lsp/);
    assert.match(found.error, /fe\.server\.path/);
  } finally {
    process.env.PATH = previous;
  }
});

test("settings reach the server in the shape it reads them", () => {
  reset({
    fe: {
      manifest: "aircraft/fe.toml",
      "inlayHints.enable": false,
      "semanticTokens.enable": true,
    },
  });
  assert.deepEqual(extension.initializationOptions(), {
    manifest: "aircraft/fe.toml",
    inlayHints: { enable: false },
    semanticTokens: { enable: true },
  });
});

test("an unset manifest is sent as empty rather than undefined", () => {
  reset({ fe: {} });
  assert.equal(extension.initializationOptions().manifest, "");
});

test("activating starts the client and watches both file kinds", async () => {
  const { directory } = withFakeServer();
  reset({ fe: { "server.path": "" } });
  const previous = process.env.PATH;
  process.env.PATH = directory;
  try {
    const { client } = await extension.activate(context());
    assert.ok(client.started, "the client should have been started");
    assert.deepEqual(client.clientOptions.documentSelector, [
      { scheme: "file", language: "fe" },
    ]);
    // A `.fe` file changed outside the editor, and a change to the aircraft's
    // symbols, both have to re-check the project.
    assert.deepEqual(state.watchers, ["**/*.fe", "**/fe.toml"]);
  } finally {
    process.env.PATH = previous;
  }
});

test("a missing server is reported rather than silently doing less", async () => {
  reset({ fe: { "server.path": "" } });
  const previous = process.env.PATH;
  process.env.PATH = "";
  try {
    const { client } = await extension.activate(context());
    assert.equal(client, undefined);
    assert.equal(state.warnings.length, 1);
    assert.match(state.warnings[0], /Could not find/);
    // …and the reason is in the log too, where it can be read later.
    assert.match(state.channels[0].lines.join("\n"), /Could not find/);
  } finally {
    process.env.PATH = previous;
  }
});

test("the status bar says when only syntax is being checked", async () => {
  const { directory } = withFakeServer();
  reset({ fe: { "server.path": "" } });
  const previous = process.env.PATH;
  process.env.PATH = directory;
  try {
    const { client } = await extension.activate(context());
    const status = state.statusBars[0];

    client.emit("fe/status", {
      semantic: false,
      manifest: null,
      message: "fe: syntax-only — no fe.toml",
    });
    assert.match(status.text, /syntax only/);
    assert.equal(status.tooltip, "fe: syntax-only — no fe.toml");
    assert.equal(
      status.command,
      "fe.showOutput",
      "the reason should be reachable",
    );
    assert.ok(status.visible);

    client.emit("fe/status", {
      semantic: true,
      manifest: "/aircraft/fe.toml",
      message: "fe: checking against /aircraft/fe.toml",
    });
    assert.doesNotMatch(status.text, /syntax only/);
  } finally {
    process.env.PATH = previous;
  }
});

test("the restart command restarts the client", async () => {
  const { directory } = withFakeServer();
  reset({ fe: { "server.path": "" } });
  const previous = process.env.PATH;
  process.env.PATH = directory;
  try {
    const { client } = await extension.activate(context());
    await state.commands.get("fe.restartServer")();
    assert.equal(client.restarts, 1);
  } finally {
    process.env.PATH = previous;
  }
});

test("every contributed command is registered", async () => {
  const manifest = JSON.parse(
    fs.readFileSync(path.join(__dirname, "..", "package.json"), "utf8"),
  );
  const { directory } = withFakeServer();
  reset({ fe: { "server.path": "" } });
  const previous = process.env.PATH;
  process.env.PATH = directory;
  try {
    await extension.activate(context());
    for (const { command } of manifest.contributes.commands) {
      assert.ok(
        state.commands.has(command),
        `${command} is contributed but not registered`,
      );
    }
  } finally {
    process.env.PATH = previous;
  }
});

test("every setting the extension reads is contributed", () => {
  const manifest = JSON.parse(
    fs.readFileSync(path.join(__dirname, "..", "package.json"), "utf8"),
  );
  const contributed = Object.keys(
    manifest.contributes.configuration.properties,
  );
  for (const key of [
    "fe.server.path",
    "fe.manifest",
    "fe.inlayHints.enable",
    "fe.semanticTokens.enable",
  ]) {
    assert.ok(contributed.includes(key), `${key} is read but not contributed`);
  }
});
