mod common;

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use tempfile::TempDir;

const CALLER: &str = "---\nname: caller\ndescription: Caller skill\n---\n\
`skill::target`\n\
`ref::api/types`\n\
{> intro <}\n\
`agent::reviewer`\n\
`var::project_name`\n\
`env::api_key`\n\
`cmd::git`\n\
`skill::missing`\n";

const TARGET: &str = "---\nname: target\ndescription: Target skill\n---\n";
const AGENT: &str = "---\nname: reviewer\ndescription: Reviews code\n---\nAgent instructions.\n";

struct WorkspaceFixture {
    _directory: TempDir,
    root: PathBuf,
    caller_path: PathBuf,
}

struct LspClient {
    child: Child,
    input: Option<BufWriter<ChildStdin>>,
    output: BufReader<ChildStdout>,
    next_id: u64,
}

impl WorkspaceFixture {
    fn new() -> Self {
        let directory = TempDir::new().unwrap();
        let root = directory.path().to_path_buf();
        let init = common::run_skillet(&root, &["init"]);
        assert!(
            init.status.success(),
            "skillet init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );

        let config_path = root.join("skillet.toml");
        let mut config = std::fs::read_to_string(&config_path).unwrap();
        config.push_str(
            "\nagents_dir = \"src/agents\"\n\
             \n[env]\napi_key = { default = \"\" }\n",
        );
        std::fs::write(config_path, config).unwrap();

        write_source(&root.join("src/skills/caller/caller.pan"), CALLER);
        write_source(&root.join("src/skills/target/target.pan"), TARGET);
        write_source(
            &root.join("src/skills/caller/references/api/types.md"),
            "# API Types\n",
        );
        write_source(
            &root.join("src/skills/_fragments/intro.fragment.pan"),
            "Shared introduction.\n",
        );
        write_source(&root.join("src/agents/reviewer.agent.pan"), AGENT);

        Self {
            caller_path: root.join("src/skills/caller/caller.pan"),
            _directory: directory,
            root,
        }
    }

    fn caller_uri(&self) -> String {
        file_uri(&self.caller_path)
    }

    fn position(&self, source: &str, needle: &str) -> Value {
        let offset = source.find(needle).expect("source contains needle");
        let prefix = &source[..offset];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
        let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
        let character = source[line_start..offset].encode_utf16().count();
        json!({ "line": line, "character": character })
    }

    fn text_position(&self, source: &str, needle: &str) -> Value {
        json!({
            "textDocument": { "uri": self.caller_uri() },
            "position": self.position(source, needle)
        })
    }
}

impl LspClient {
    fn start(root: &Path, supports_rename_file: bool) -> Self {
        let mut child = Command::new(common::skillet_bin())
            .arg("lsp")
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = BufWriter::new(child.stdin.take().unwrap());
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut client = Self {
            child,
            input: Some(input),
            output,
            next_id: 1,
        };

        let resource_operations = if supports_rename_file {
            json!(["rename"])
        } else {
            json!([])
        };
        let initialize = client.request(
            "initialize",
            json!({
                "processId": null,
                "rootUri": file_uri(root),
                "capabilities": {
                    "workspace": {
                        "workspaceEdit": {
                            "resourceOperations": resource_operations
                        }
                    }
                }
            }),
        );
        assert!(initialize.get("error").is_none(), "{initialize}");
        assert_eq!(
            initialize.pointer("/result/capabilities/definitionProvider"),
            Some(&json!(true))
        );

        client.notify("initialized", json!({}));
        client
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }));

        loop {
            let response = self.read();
            if response.get("id") == Some(&json!(id)) {
                return response;
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }));
    }

    fn shutdown(mut self) {
        let response = self.request("shutdown", Value::Null);
        assert!(response.get("error").is_none(), "{response}");
        self.notify("exit", Value::Null);
        drop(self.input.take());
        assert!(self.child.wait().unwrap().success());
    }

    fn send(&mut self, message: &Value) {
        let body = serde_json::to_vec(message).unwrap();
        let input = self.input.as_mut().unwrap();
        write!(input, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
        input.write_all(&body).unwrap();
        input.flush().unwrap();
    }

    fn read(&mut self) -> Value {
        let mut content_length = None;
        loop {
            let mut line = String::new();
            assert_ne!(
                self.output.read_line(&mut line).unwrap(),
                0,
                "server closed stdout"
            );
            if line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = Some(value.trim().parse::<usize>().unwrap());
                }
            }
        }

        let mut body = vec![0; content_length.expect("Content-Length header")];
        self.output.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        if self.child.try_wait().unwrap().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn write_source(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn file_uri(path: &Path) -> String {
    url::Url::from_file_path(path).unwrap().to_string()
}

#[test]
fn definition_resolves_workspace_symbols() {
    let workspace = WorkspaceFixture::new();
    let mut client = LspClient::start(&workspace.root, true);

    let cases = [
        ("skill::target", "src/skills/target/target.pan"),
        (
            "ref::api/types",
            "src/skills/caller/references/api/types.md",
        ),
        ("{> intro <", "src/skills/_fragments/intro.fragment.pan"),
        ("agent::reviewer", "src/agents/reviewer.agent.pan"),
        ("var::project_name", "skillet.toml"),
        ("env::api_key", "skillet.toml"),
    ];

    for (needle, expected_path) in cases {
        let response = client.request(
            "textDocument/definition",
            workspace.text_position(CALLER, needle),
        );
        let result = response
            .pointer("/result/0/targetUri")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("definition failed for {needle}: {response}"));
        assert!(
            result.ends_with(expected_path),
            "{needle} resolved to {result}, expected {expected_path}"
        );
    }

    let unresolved = client.request(
        "textDocument/definition",
        workspace.text_position(CALLER, "skill::missing"),
    );
    assert!(unresolved.get("result").is_some());
    assert!(unresolved["result"].is_null());

    client.shutdown();
}

#[test]
fn prepare_rename_accepts_supported_refs_and_rejects_others() {
    let workspace = WorkspaceFixture::new();
    let mut client = LspClient::start(&workspace.root, true);

    for needle in [
        "skill::target",
        "ref::api/types",
        "{> intro <}",
        "agent::reviewer",
    ] {
        let response = client.request(
            "textDocument/prepareRename",
            workspace.text_position(CALLER, needle),
        );
        assert!(
            response.get("error").is_none(),
            "prepareRename failed for {needle}: {response}"
        );
        assert_eq!(response["result"]["placeholder"], needle_name(needle));
    }

    for needle in ["cmd::git", "skill::missing"] {
        let response = client.request(
            "textDocument/prepareRename",
            workspace.text_position(CALLER, needle),
        );
        assert!(
            response.get("error").is_some(),
            "prepareRename unexpectedly accepted {needle}: {response}"
        );
    }

    client.shutdown();
}

#[test]
fn rename_emits_text_edits_before_file_operations_and_uses_open_text() {
    let workspace = WorkspaceFixture::new();
    let mut client = LspClient::start(&workspace.root, true);
    let changed_caller = format!("{CALLER}`skill::target`\n");
    let caller_uri = workspace.caller_uri();
    client.notify(
        "textDocument/didOpen",
        json!({
            "textDocument": { "uri": caller_uri, "languageId": "pan", "version": 1, "text": CALLER }
        }),
    );
    client.notify(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": caller_uri, "version": 2 },
            "contentChanges": [{ "text": changed_caller }]
        }),
    );

    let cases = [
        (
            "skill::target",
            "renamed-skill",
            "src/skills/target",
            "src/skills/renamed-skill",
        ),
        (
            "ref::api/types",
            "api/models",
            "src/skills/caller/references/api/types.md",
            "src/skills/caller/references/api/models.md",
        ),
        (
            "{> intro <}",
            "opening",
            "src/skills/_fragments/intro.fragment.pan",
            "src/skills/_fragments/opening.fragment.pan",
        ),
        (
            "agent::reviewer",
            "editor",
            "src/agents/reviewer.agent.pan",
            "src/agents/editor.agent.pan",
        ),
    ];

    for (needle, new_name, old_path, new_path) in cases {
        let source = if needle == "skill::target" {
            changed_caller.as_str()
        } else {
            CALLER
        };
        let mut params = workspace.text_position(source, needle);
        params["newName"] = json!(new_name);
        let response = client.request("textDocument/rename", params);
        let operations = response
            .pointer("/result/documentChanges")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("rename failed for {needle}: {response}"));

        let first_rename = operations
            .iter()
            .position(|operation| operation["kind"] == "rename");
        if let Some(first_rename) = first_rename {
            assert!(
                operations[..first_rename]
                    .iter()
                    .all(|operation| operation.get("textDocument").is_some()),
                "text edits must precede file renames: {operations:?}"
            );
            assert!(
                operations[first_rename..]
                    .iter()
                    .all(|operation| operation["kind"] == "rename"),
                "file renames must follow text edits: {operations:?}"
            );
        }

        let expected_uri = workspace.caller_uri();
        assert!(
            operations.iter().any(|operation| {
                operation["textDocument"]["uri"].as_str() == Some(expected_uri.as_str())
            }),
            "{needle} plan did not include expected source edit: {operations:?}"
        );

        let rename_operations = operations
            .iter()
            .filter(|operation| operation["kind"] == "rename")
            .collect::<Vec<_>>();
        let last_rename = rename_operations.last().unwrap();
        assert!(
            last_rename["oldUri"]
                .as_str()
                .is_some_and(|uri| uri.ends_with(old_path)),
            "{needle} file rename missing: {operations:?}"
        );
        assert!(
            last_rename["newUri"]
                .as_str()
                .is_some_and(|uri| uri.ends_with(new_path)),
            "{needle} rename target missing: {operations:?}"
        );

        if needle == "skill::target" {
            assert_eq!(rename_operations.len(), 2);
            assert!(rename_operations[0]["oldUri"]
                .as_str()
                .unwrap()
                .ends_with("src/skills/target/target.pan"));
            assert!(rename_operations[0]["newUri"]
                .as_str()
                .unwrap()
                .ends_with("src/skills/target/renamed-skill.pan"));
            let caller_edits = operations
                .iter()
                .find(|operation| operation["textDocument"]["uri"] == caller_uri)
                .unwrap();
            assert_eq!(caller_edits["edits"].as_array().unwrap().len(), 2);
        }

        if needle == "agent::reviewer" {
            let agent_uri = file_uri(&workspace.root.join("src/agents/reviewer.agent.pan"));
            let agent_edits = operations
                .iter()
                .find(|operation| {
                    operation["textDocument"]["uri"].as_str() == Some(agent_uri.as_str())
                })
                .unwrap();
            assert!(agent_edits["edits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|edit| edit["newText"] == "editor"));
        }
    }

    client.shutdown();
}

#[test]
fn rename_requires_client_rename_file_capability() {
    let workspace = WorkspaceFixture::new();
    let mut client = LspClient::start(&workspace.root, false);
    let mut params = workspace.text_position(CALLER, "skill::target");
    params["newName"] = json!("renamed");

    let response = client.request("textDocument/rename", params);
    assert_eq!(response["error"]["code"], -32600);
    assert!(response["error"]["message"]
        .as_str()
        .unwrap()
        .contains("RenameFile"));

    client.shutdown();
}

#[test]
fn save_and_close_update_open_document_overlays() {
    let workspace = WorkspaceFixture::new();
    let mut client = LspClient::start(&workspace.root, true);
    let caller_uri = workspace.caller_uri();
    let changed = format!("{CALLER}`skill::target`\n");
    let saved = format!("{changed}`skill::target`\n");

    client.notify(
        "textDocument/didOpen",
        json!({
            "textDocument": { "uri": caller_uri, "languageId": "pan", "version": 1, "text": CALLER }
        }),
    );
    client.notify(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": caller_uri, "version": 2 },
            "contentChanges": [{ "text": changed }]
        }),
    );
    client.notify(
        "textDocument/didSave",
        json!({ "textDocument": { "uri": caller_uri } }),
    );
    assert_eq!(caller_edit_count(&mut client, &workspace, CALLER), 1);

    client.notify(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": caller_uri, "version": 3 },
            "contentChanges": [{ "text": changed }]
        }),
    );
    client.notify(
        "textDocument/didSave",
        json!({
            "textDocument": { "uri": caller_uri },
            "text": saved
        }),
    );
    assert_eq!(caller_edit_count(&mut client, &workspace, &saved), 3);

    client.notify(
        "textDocument/didClose",
        json!({ "textDocument": { "uri": caller_uri } }),
    );
    assert_eq!(caller_edit_count(&mut client, &workspace, CALLER), 1);

    client.shutdown();
}

fn caller_edit_count(client: &mut LspClient, workspace: &WorkspaceFixture, source: &str) -> usize {
    let mut params = workspace.text_position(source, "skill::target");
    params["newName"] = json!("renamed");
    let response = client.request("textDocument/rename", params);
    response["result"]["documentChanges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|change| change["textDocument"]["uri"] == workspace.caller_uri())
        .unwrap()["edits"]
        .as_array()
        .unwrap()
        .len()
}

fn needle_name(needle: &str) -> &str {
    if needle.starts_with("{>") {
        "intro"
    } else {
        needle.split_once("::").unwrap().1
    }
}
