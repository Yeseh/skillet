//! LSP transport and workspace integration.
//!
//! Filesystem access stays in this CLI adapter as required by
//! [ADR-0004](../../../../docs/adr/0004-lib-cli-io-boundary.md). Symbol
//! resolution and plugin scoping remain in `skillet::navigation`, following
//! [ADR-0005](../../../../docs/adr/0005-multi-plugin-workspace.md).

mod position;
mod state;

use anyhow::{bail, Context, Result};
use lsp_server::{Connection, Message, Request, Response};
use lsp_types::Position;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use skillet::navigation::{self, ByteRange, RenameError, WorkspaceSnapshot};
use std::path::{Path, PathBuf};

use self::position::{byte_range_to_lsp_range, lsp_position_to_byte_offset};
use self::state::LspState;

/// Runs the synchronous stdio Language Server Protocol server.
pub fn run() -> Result<()> {
    let (connection, io_threads) = Connection::stdio();
    let mut state = LspState::new();
    let initialize = connection
        .receiver
        .recv()
        .context("failed to receive LSP initialize request")?;

    let Message::Request(request) = initialize else {
        bail!("expected an LSP initialize request");
    };
    if request.method != "initialize" {
        bail!("expected an LSP initialize request");
    }

    state.set_client_supports_rename_file(client_supports_rename_file(&request.params));
    connection.sender.send(Message::Response(Response::new_ok(
        request.id,
        json!({
            "capabilities": {
                "definitionProvider": true,
                "renameProvider": { "prepareProvider": true },
                "textDocumentSync": 1
            },
            "serverInfo": {
                "name": "skillet",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
    )))?;

    loop {
        match connection.receiver.recv()? {
            Message::Request(request) => {
                let response = handle_request(&mut state, request);
                connection.sender.send(Message::Response(response))?;
            }
            Message::Notification(notification) => {
                if notification.method == "exit" {
                    break;
                }
                if let Err(error) =
                    handle_notification(&mut state, notification.method, notification.params)
                {
                    eprintln!("LSP notification failed: {error:#}");
                }
            }
            Message::Response(_) => {}
        }
    }

    drop(connection);
    io_threads.join()?;
    Ok(())
}

fn client_supports_rename_file(params: &Value) -> bool {
    params
        .pointer("/capabilities/workspace/workspaceEdit/resourceOperations")
        .and_then(Value::as_array)
        .is_some_and(|operations| operations.iter().any(|operation| operation == "rename"))
}

fn handle_request(state: &mut LspState, request: Request) -> Response {
    let Request {
        id, method, params, ..
    } = request;

    if method == "shutdown" {
        return Response::new_ok(id, Value::Null);
    }

    let result = process_request(state, &method, params);
    match result {
        Ok(value) => Response::new_ok(id, value),
        Err(error) => Response::new_err(id, error.code, error.message),
    }
}

fn process_request(
    state: &mut LspState,
    method: &str,
    params: Value,
) -> std::result::Result<Value, RpcError> {
    match method {
        "textDocument/definition" => definition(state, params),
        "textDocument/prepareRename" => prepare_rename(state, params),
        "textDocument/rename" => rename(state, params),
        _ => Err(RpcError::new(-32601, format!("method not found: {method}"))),
    }
}

fn definition(state: &LspState, params: Value) -> std::result::Result<Value, RpcError> {
    let params: PositionParams = parse_params(params)?;
    let path = uri_to_path(&params.text_document.uri)?;
    let Some(snapshot) = state.build_snapshot_for(&path)? else {
        return Ok(Value::Null);
    };
    let Some(source) = source_for(&snapshot, &path) else {
        return Ok(Value::Null);
    };
    let Some(byte_offset) = lsp_position_to_byte_offset(source, params.position) else {
        return Err(RpcError::invalid_position());
    };
    let Some(symbol) = navigation::symbol_at_offset(&snapshot, &path, byte_offset) else {
        return Ok(Value::Null);
    };
    let Some(target) = navigation::resolve_definition(&snapshot, &path, byte_offset) else {
        return Ok(Value::Null);
    };

    let origin_range = range_value(source, symbol.full_range)?;
    let target_range = match target.target_range {
        Some(range) => {
            let target_source = source_for(&snapshot, &target.path).ok_or_else(|| {
                RpcError::internal_message("definition range is not in a workspace source")
            })?;
            range_value(target_source, range)?
        }
        None => serde_json::to_value(lsp_types::Range::new(
            Position::new(0, 0),
            Position::new(0, 0),
        ))
        .map_err(RpcError::internal)?,
    };
    let target_uri = path_to_uri(&target.path)?;

    Ok(json!([{
        "originSelectionRange": origin_range,
        "targetUri": target_uri,
        "targetRange": target_range.clone(),
        "targetSelectionRange": target_range
    }]))
}

fn prepare_rename(state: &LspState, params: Value) -> std::result::Result<Value, RpcError> {
    let params: PositionParams = parse_params(params)?;
    let path = uri_to_path(&params.text_document.uri)?;
    let snapshot = snapshot_for_request(state, &path)?;
    let source = source_for(&snapshot, &path)
        .ok_or_else(|| RpcError::request_failed("document is not a workspace skill or agent"))?;
    let byte_offset = position_offset(source, params.position)?;
    let symbol = navigation::prepare_rename(&snapshot, &path, byte_offset).map_err(rename_error)?;
    let range = range_value(source, symbol.value_range)?;

    Ok(json!({
        "range": range,
        "placeholder": symbol.name
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenameParams {
    #[serde(flatten)]
    text_document_position: PositionParams,
    new_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PositionParams {
    text_document: TextDocumentIdentifier,
    position: Position,
}

#[derive(Deserialize)]
struct TextDocumentIdentifier {
    uri: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DidOpenParams {
    text_document: OpenTextDocument,
}

#[derive(Deserialize)]
struct OpenTextDocument {
    uri: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DidChangeParams {
    text_document: TextDocumentIdentifier,
    content_changes: Vec<ContentChange>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DidCloseParams {
    text_document: TextDocumentIdentifier,
}

#[derive(Deserialize)]
struct ContentChange {
    text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DidSaveParams {
    text_document: TextDocumentIdentifier,
    text: Option<String>,
}

fn rename(state: &LspState, params: Value) -> std::result::Result<Value, RpcError> {
    if !state.client_supports_rename_file() {
        return Err(RpcError::new(
            -32600,
            "client does not support RenameFile resource operations",
        ));
    }

    let params: RenameParams = parse_params(params)?;
    let path = uri_to_path(&params.text_document_position.text_document.uri)?;
    let snapshot = snapshot_for_request(state, &path)?;
    let source = source_for(&snapshot, &path)
        .ok_or_else(|| RpcError::request_failed("document is not a workspace skill or agent"))?;
    let byte_offset = position_offset(source, params.text_document_position.position)?;
    let plan = navigation::plan_rename(&snapshot, &path, byte_offset, &params.new_name)
        .map_err(rename_error)?;

    let mut document_changes = Vec::new();
    for file_edits in plan.text_edits {
        let source = source_for(&snapshot, &file_edits.path).ok_or_else(|| {
            RpcError::internal_message(format!(
                "rename plan refers to an unknown source file '{}'",
                file_edits.path.display()
            ))
        })?;
        let edits = file_edits
            .edits
            .into_iter()
            .map(|edit| {
                Ok(json!({
                    "range": range_value(source, edit.range)?,
                    "newText": edit.new_text
                }))
            })
            .collect::<std::result::Result<Vec<_>, RpcError>>()?;
        document_changes.push(json!({
            "textDocument": {
                "uri": path_to_uri(&file_edits.path)?,
                "version": Value::Null
            },
            "edits": edits
        }));
    }

    for file_rename in plan.file_renames {
        document_changes.push(json!({
            "kind": "rename",
            "oldUri": path_to_uri(&file_rename.old_path)?,
            "newUri": path_to_uri(&file_rename.new_path)?
        }));
    }

    Ok(json!({ "documentChanges": document_changes }))
}

fn snapshot_for_request(
    state: &LspState,
    path: &Path,
) -> std::result::Result<WorkspaceSnapshot, RpcError> {
    state
        .build_snapshot_for(path)?
        .ok_or_else(|| RpcError::request_failed("could not find skillet.toml for document"))
}

fn source_for<'a>(snapshot: &'a WorkspaceSnapshot, path: &Path) -> Option<&'a str> {
    if snapshot.config_path == path {
        return Some(&snapshot.config_raw);
    }
    snapshot
        .skills
        .iter()
        .find(|skill| skill.path == path)
        .map(|skill| skill.source.as_str())
        .or_else(|| {
            snapshot
                .agents
                .iter()
                .find(|agent| agent.path == path)
                .map(|agent| agent.source.as_str())
        })
}

fn position_offset(source: &str, position: Position) -> std::result::Result<usize, RpcError> {
    lsp_position_to_byte_offset(source, position).ok_or_else(RpcError::invalid_position)
}

fn range_value(source: &str, range: ByteRange) -> std::result::Result<Value, RpcError> {
    let range = byte_range_to_lsp_range(source, range)
        .ok_or_else(|| RpcError::internal_message("navigation returned an invalid source range"))?;
    serde_json::to_value(range).map_err(RpcError::internal)
}

fn uri_to_path(uri: &str) -> std::result::Result<PathBuf, RpcError> {
    let uri = url::Url::parse(uri)
        .map_err(|_| RpcError::invalid_params("document URI must be a valid file URI"))?;
    uri.to_file_path()
        .map_err(|_| RpcError::invalid_params("document URI must use the file scheme"))
}

fn path_to_uri(path: &Path) -> std::result::Result<String, RpcError> {
    url::Url::from_file_path(path)
        .map(|uri| uri.to_string())
        .map_err(|_| {
            RpcError::internal_message(format!(
                "could not convert '{}' to a file URI",
                path.display()
            ))
        })
}

fn parse_params<T: DeserializeOwned>(params: Value) -> std::result::Result<T, RpcError> {
    serde_json::from_value(params)
        .map_err(|error| RpcError::invalid_params(format!("invalid request parameters: {error}")))
}

fn rename_error(error: RenameError) -> RpcError {
    let message = match error {
        RenameError::NoSymbolAtCursor => {
            "there is no renameable symbol at this position".to_string()
        }
        RenameError::NotRenameable { reason, .. } => reason,
        RenameError::UnresolvedTarget { kind, name } => {
            format!("unresolved {kind:?} symbol '{name}' cannot be renamed")
        }
        RenameError::InvalidName { reason, .. } => format!("invalid new name: {reason}"),
        RenameError::AlreadyExists { path } => {
            format!("rename target '{}' already exists", path.display())
        }
        RenameError::NoOp => "new name is unchanged".to_string(),
    };
    RpcError::request_failed(message)
}

fn handle_notification(state: &mut LspState, method: String, params: Value) -> Result<()> {
    match method.as_str() {
        "textDocument/didOpen" => {
            let params: DidOpenParams = serde_json::from_value(params)?;
            state.did_open(
                uri_to_path(&params.text_document.uri)
                    .map_err(|error| anyhow::anyhow!(error.message))?,
                params.text_document.text,
            );
        }
        "textDocument/didChange" => {
            let params: DidChangeParams = serde_json::from_value(params)?;
            if let Some(change) = params.content_changes.into_iter().last() {
                state.did_change(
                    uri_to_path(&params.text_document.uri)
                        .map_err(|error| anyhow::anyhow!(error.message))?,
                    change.text,
                );
            }
        }
        "textDocument/didClose" => {
            let params: DidCloseParams = serde_json::from_value(params)?;
            let path = uri_to_path(&params.text_document.uri)
                .map_err(|error| anyhow::anyhow!(error.message))?;
            state.did_close(&path);
        }
        "textDocument/didSave" => {
            let params: DidSaveParams = serde_json::from_value(params)?;
            let path = uri_to_path(&params.text_document.uri)
                .map_err(|error| anyhow::anyhow!(error.message))?;
            state.did_save(path, params.text);
        }
        _ => {}
    }

    Ok(())
}

#[derive(Debug)]
struct RpcError {
    code: i32,
    message: String,
}

impl RpcError {
    fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn invalid_position() -> Self {
        Self::invalid_params("position is outside the document or splits a UTF-16 surrogate pair")
    }

    fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(-32602, message)
    }

    fn request_failed(message: impl Into<String>) -> Self {
        Self::new(-32803, message)
    }

    fn internal(error: serde_json::Error) -> Self {
        eprintln!("LSP response serialization failed: {error}");
        Self::internal_message("failed to serialize LSP response")
    }

    fn internal_message(message: impl Into<String>) -> Self {
        Self::new(-32603, message)
    }
}

impl From<anyhow::Error> for RpcError {
    fn from(error: anyhow::Error) -> Self {
        eprintln!("LSP request failed: {error:#}");
        Self::internal_message("failed to build workspace snapshot")
    }
}
