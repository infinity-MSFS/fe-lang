//! `fe-lsp` — the language server, over stdio.

use lsp_server::Connection;
use lsp_types::InitializeParams;

fn main() -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("fe-lsp {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                print!("{USAGE}");
                return Ok(());
            }
            "--stdio" => {}
            other => {
                eprintln!("fe-lsp: unrecognised argument `{other}`\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let (connection, threads) = Connection::stdio();
    let (id, params) = connection.initialize_start()?;
    let params: InitializeParams = serde_json::from_value(params)?;

    let encoding = fe_lsp::line_index::Encoding::negotiate(
        params
            .capabilities
            .general
            .as_ref()
            .and_then(|general| general.position_encodings.as_deref()),
    );
    connection.initialize_finish(
        id,
        serde_json::json!({
            "capabilities": fe_lsp::capabilities(encoding),
            "serverInfo": { "name": "fe-lsp", "version": env!("CARGO_PKG_VERSION") },
        }),
    )?;

    fe_lsp::Server::new(connection, params).run()?;
    threads.join()?;
    Ok(())
}

const USAGE: &str = "\
fe-lsp — language server for the FE procedure language

USAGE:
    fe-lsp [--stdio]

The server communicates over stdin and stdout; an editor starts it, not a
person. It checks a project against the `fe.toml` above it, and reports syntax
only when there is no such file.

OPTIONS:
    --stdio          Accepted and ignored; stdio is the only transport.
    -V, --version    Print the version.
    -h, --help       Print this message.

The server writes nothing to stdout but protocol messages. Both editors capture
its stderr into their own log, which is where a panic will appear.
";
