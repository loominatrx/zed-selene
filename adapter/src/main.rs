//! LSP bridge that runs Selene against the current unsaved Luau document.

use lsp_server::{Connection, Message, Notification, Response};
use lsp_types::{
    notification::{
        DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument,
        Notification as LspNotification,
    },
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, InitializeParams, InitializeResult, NumberOrString, Position,
    PublishDiagnosticsParams, Range, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, Uri,
};
use serde_json::Value;
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

fn main() -> anyhowless::Result<()> {
    let (connection, io_threads) = Connection::stdio();
    let (id, params) = connection.initialize_start()?;
    let _params: InitializeParams = serde_json::from_value(params)?;
    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        ..Default::default()
    };
    connection.initialize_finish(
        id,
        serde_json::to_value(InitializeResult {
            capabilities,
            server_info: None,
        })?,
    )?;

    let mut root = std::env::current_dir()?;
    for message in &connection.receiver {
        match message {
            Message::Request(request) if request.method == "shutdown" => {
                connection.handle_shutdown(&request)?;
                break;
            }
            Message::Notification(notification) => {
                if let Err(error) = handle_notification(&connection, &notification, &mut root) {
                    eprintln!("selene LSP: {error}");
                }
            }
            Message::Request(request) => {
                connection.sender.send(Message::Response(Response::new_err(
                    request.id,
                    lsp_server::ErrorCode::MethodNotFound as i32,
                    "unsupported request".into(),
                )))?;
            }
            Message::Response(_) => {}
        }
    }
    // Close the channels before joining the stdio reader/writer threads.
    drop(connection);
    io_threads.join()?;
    Ok(())
}

fn handle_notification(
    connection: &Connection,
    notification: &Notification,
    root: &mut PathBuf,
) -> anyhowless::Result<()> {
    match notification.method.as_str() {
        "initialized" => {
            // Zed's worktree is the process working directory.
        }
        DidOpenTextDocument::METHOD => {
            let params: DidOpenTextDocumentParams =
                serde_json::from_value(notification.params.clone())?;
            lint(
                connection,
                root,
                params.text_document.uri,
                params.text_document.text,
            )?;
        }
        DidChangeTextDocument::METHOD => {
            let params: DidChangeTextDocumentParams =
                serde_json::from_value(notification.params.clone())?;
            if let Some(change) = params.content_changes.last() {
                lint(
                    connection,
                    root,
                    params.text_document.uri,
                    change.text.clone(),
                )?;
            }
        }
        DidCloseTextDocument::METHOD => {
            let params: DidCloseTextDocumentParams =
                serde_json::from_value(notification.params.clone())?;
            connection
                .sender
                .send(Message::Notification(Notification::new(
                    "textDocument/publishDiagnostics".into(),
                    PublishDiagnosticsParams {
                        uri: params.text_document.uri,
                        diagnostics: vec![],
                        version: None,
                    },
                )))?;
        }
        _ => {}
    }
    Ok(())
}

fn lint(connection: &Connection, root: &PathBuf, uri: Uri, text: String) -> anyhowless::Result<()> {
    let selene = std::env::var_os("SELENE_PATH").unwrap_or_else(|| "selene".into());
    let mut child = Command::new(selene)
        .current_dir(root)
        .args(["--display-style=json2", "--no-summary", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(text.as_bytes())?;
    let output = child.wait_with_output()?;
    let mut diagnostics = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let value: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value["type"] != "Diagnostic" {
            continue;
        }
        let label = &value["primary_label"];
        let span = &label["span"];
        // Selene's JSON2 spans are zero-based, matching LSP coordinates.
        let line_number = span["start_line"].as_u64().unwrap_or(0) as u32;
        let column = span["start_column"].as_u64().unwrap_or(0) as u32;
        let end_line = span["end_line"].as_u64().unwrap_or(line_number as u64) as u32;
        let end_column = span["end_column"].as_u64().unwrap_or(column as u64) as u32;
        let severity = match value["severity"]
            .as_str()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "error" => Some(DiagnosticSeverity::ERROR),
            "warning" => Some(DiagnosticSeverity::WARNING),
            _ => Some(DiagnosticSeverity::INFORMATION),
        };
        diagnostics.push(Diagnostic {
            range: Range {
                start: Position {
                    line: line_number,
                    character: column,
                },
                end: Position {
                    line: end_line,
                    character: end_column,
                },
            },
            severity,
            code: value["code"]
                .as_str()
                .map(|s| NumberOrString::String(s.to_string())),
            source: Some("selene".into()),
            message: value["message"]
                .as_str()
                .unwrap_or("Selene diagnostic")
                .to_string(),
            ..Default::default()
        });
    }
    connection
        .sender
        .send(Message::Notification(Notification::new(
            "textDocument/publishDiagnostics".into(),
            PublishDiagnosticsParams {
                uri,
                diagnostics,
                version: None,
            },
        )))?;
    Ok(())
}

// A small local error alias keeps the adapter's error conversions explicit.
mod anyhowless {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
}
