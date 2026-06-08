import * as vscode from "vscode";
import * as cp from "child_process";
import * as path from "path";
import * as fs from "fs";

// ── Types ─────────────────────────────────────────────────────────────────────

interface LintDiagnostic {
  rule: string;
  severity: "error" | "warning" | "info";
  skill: string;
  message: string;
  path?: string;
  line?: number;
}

// ── Module-level state ────────────────────────────────────────────────────────

let outputChannel: vscode.OutputChannel;
const diagnosticsCollection =
  vscode.languages.createDiagnosticCollection("skillet");

/** Whether we've already shown the "skillet not found" warning this session. */
let cliMissingWarned = false;

/** Per-document debounce timers. */
const debounceTimers = new Map<string, ReturnType<typeof setTimeout>>();

/** Per-document in-flight lint processes (for cancellation). */
const inFlightProcesses = new Map<string, cp.ChildProcess>();

// ── Typed-ref decoration ──────────────────────────────────────────────────────

const REF_DECORATION = vscode.window.createTextEditorDecorationType({
  backgroundColor: new vscode.ThemeColor("badge.background"),
  color: new vscode.ThemeColor("badge.foreground"),
  borderRadius: "3px",
  // Small padding improves pill shape readability
  before: { margin: "0 1px" },
  after: { margin: "0 1px" },
});

const TYPED_REF_RE = /((?<!`)`(?!`))(ref|cmd|skill|var|env|agent)(::)([^`]*?)((?<!`)`(?!`))/g;

function applyRefDecorations(editor: vscode.TextEditor): void {
  const ranges: vscode.Range[] = [];
  const text = editor.document.getText();
  let match: RegExpExecArray | null;
  TYPED_REF_RE.lastIndex = 0;
  while ((match = TYPED_REF_RE.exec(text)) !== null) {
    const start = editor.document.positionAt(match.index);
    const end = editor.document.positionAt(match.index + match[0].length);
    ranges.push(new vscode.Range(start, end));
  }
  editor.setDecorations(REF_DECORATION, ranges);
}

// ── Workspace root detection ──────────────────────────────────────────────────

/**
 * Walk up from `filePath` to find the nearest directory containing
 * `skillet.toml`.  Returns `undefined` if not found.
 */
function findWorkspaceRoot(filePath: string): string | undefined {
  let dir = path.dirname(filePath);
  const root = path.parse(dir).root;
  while (true) {
    if (fs.existsSync(path.join(dir, "skillet.toml"))) {
      return dir;
    }
    if (dir === root) {
      return undefined;
    }
    dir = path.dirname(dir);
  }
}

// ── Settings helpers ──────────────────────────────────────────────────────────

function executablePath(workspaceRoot: string): string {
  const cfg = vscode.workspace.getConfiguration("skillet");
  const raw = cfg.get<string>("executablePath", "skillet");
  if (path.isAbsolute(raw)) {
    return raw;
  }
  // Relative path: resolve against workspace root
  if (raw !== "skillet" && raw !== "") {
    return path.resolve(workspaceRoot, raw);
  }
  return raw;
}

function isPedantic(): boolean {
  return vscode.workspace
    .getConfiguration("skillet")
    .get<boolean>("pedantic", false);
}

// ── CLI invocation ────────────────────────────────────────────────────────────

function runLint(
  key: string,
  workspaceRoot: string
): void {
  const docKey = key;

  // Cancel any existing in-flight process for this document
  const existing = inFlightProcesses.get(docKey);
  if (existing) {
    existing.kill();
    inFlightProcesses.delete(docKey);
  }

  const exe = executablePath(workspaceRoot);
  const args = ["lint", "--format", "json"];
  if (isPedantic()) {
    args.push("--pedantic");
  }

  let stdout = "";
  let stderr = "";

  const proc = cp.spawn(exe, args, { cwd: workspaceRoot });
  inFlightProcesses.set(docKey, proc);

  proc.stdout.on("data", (chunk: Buffer) => {
    stdout += chunk.toString();
  });
  proc.stderr.on("data", (chunk: Buffer) => {
    stderr += chunk.toString();
  });

  proc.on("error", (err: NodeJS.ErrnoException) => {
    inFlightProcesses.delete(docKey);
    if (err.code === "ENOENT") {
      warnCliMissing(exe);
    } else {
      outputChannel.appendLine(
        `[error] Failed to spawn '${exe}': ${err.message}`
      );
      outputChannel.show(true);
    }
  });

  proc.on("close", (code: number | null) => {
    inFlightProcesses.delete(docKey);

    if (stderr.trim()) {
      outputChannel.appendLine(
        `[skillet lint] exit ${code ?? "?"} — stderr:\n${stderr.trim()}`
      );
      outputChannel.show(true);
      return;
    }

    let diagnostics: LintDiagnostic[];
    try {
      diagnostics = JSON.parse(stdout) as LintDiagnostic[];
    } catch {
      if (stdout.trim()) {
        outputChannel.appendLine(
          `[skillet lint] exit ${code ?? "?"} — invalid JSON output:\n${stdout.trim()}`
        );
        outputChannel.show(true);
      }
      return;
    }

    publishDiagnostics(diagnostics, workspaceRoot);
  });
}

// ── Diagnostics publishing ────────────────────────────────────────────────────

function publishDiagnostics(
  lintDiags: LintDiagnostic[],
  workspaceRoot: string
): void {
  // Group by file path; diagnostics without a path go to a synthetic URI
  const byUri = new Map<string, vscode.Diagnostic[]>();

  for (const d of lintDiags) {
    const uriStr = d.path
      ? vscode.Uri.file(
          path.isAbsolute(d.path) ? d.path : path.join(workspaceRoot, d.path)
        ).toString()
      : vscode.Uri.file(path.join(workspaceRoot, "skillet.toml")).toString();

    const vscodeDiag = toVscodeDiagnostic(d);
    const existing = byUri.get(uriStr) ?? [];
    existing.push(vscodeDiag);
    byUri.set(uriStr, existing);
  }

  // Apply — replace all diagnostics for each affected file
  diagnosticsCollection.clear();
  for (const [uriStr, diags] of byUri.entries()) {
    diagnosticsCollection.set(vscode.Uri.parse(uriStr), diags);
  }
}

function toVscodeDiagnostic(d: LintDiagnostic): vscode.Diagnostic {
  const severity =
    d.severity === "error"
      ? vscode.DiagnosticSeverity.Error
      : d.severity === "warning"
      ? vscode.DiagnosticSeverity.Warning
      : vscode.DiagnosticSeverity.Information;

  // line is 1-based from skillet; VS Code uses 0-based
  const line = d.line !== undefined ? Math.max(0, d.line - 1) : 0;
  const range = new vscode.Range(line, 0, line, Number.MAX_SAFE_INTEGER);

  const diag = new vscode.Diagnostic(
    range,
    `${d.message} [${d.rule}]`,
    severity
  );
  diag.source = "skillet";
  diag.code = d.rule;
  return diag;
}

// ── CLI-not-found warning ─────────────────────────────────────────────────────

function warnCliMissing(exe: string): void {
  if (cliMissingWarned) {
    return;
  }
  cliMissingWarned = true;
  vscode.window
    .showWarningMessage(
      `Skillet: '${exe}' not found. Install skillet or set skillet.executablePath.`,
      "Don't show again"
    )
    .then((choice) => {
      if (choice === "Don't show again") {
        // Already suppressed for this session via the flag; nothing to persist.
      }
    });
}

// ── Save handler with debounce ────────────────────────────────────────────────

const DEBOUNCE_MS = 400;

function onDidSaveTextDocument(document: vscode.TextDocument): void {
  if (document.languageId !== "pan") {
    return;
  }

  const docKey = document.uri.toString();
  const existing = debounceTimers.get(docKey);
  if (existing) {
    clearTimeout(existing);
  }

  const timer = setTimeout(() => {
    debounceTimers.delete(docKey);
    const workspaceRoot = findWorkspaceRoot(document.uri.fsPath);
    if (!workspaceRoot) {
      // No skillet.toml found — silently skip
      return;
    }
    runLint(document.uri.toString(), workspaceRoot);
  }, DEBOUNCE_MS);

  debounceTimers.set(docKey, timer);
}

// ── Activation ────────────────────────────────────────────────────────────────

export function activate(context: vscode.ExtensionContext): void {
  outputChannel = vscode.window.createOutputChannel("Skillet");

  // Decorate currently open .pan editors on activation
  for (const editor of vscode.window.visibleTextEditors) {
    if (editor.document.languageId === "pan") {
      applyRefDecorations(editor);
    }
  }

  // Decorate on editor switch
  context.subscriptions.push(
    vscode.window.onDidChangeActiveTextEditor((editor) => {
      if (editor && editor.document.languageId === "pan") {
        applyRefDecorations(editor);
      }
    })
  );

  // Re-decorate on document change (typing)
  context.subscriptions.push(
    vscode.workspace.onDidChangeTextDocument((event) => {
      const editor = vscode.window.activeTextEditor;
      if (
        editor &&
        editor.document === event.document &&
        event.document.languageId === "pan"
      ) {
        applyRefDecorations(editor);
      }
    })
  );

  // Lint on save
  context.subscriptions.push(
    vscode.workspace.onDidSaveTextDocument(onDidSaveTextDocument)
  );

  // Re-run lint when SKILL.md files change (e.g. after `skillet build`)
  const skillWatcher = vscode.workspace.createFileSystemWatcher("**/SKILL.md");
  const onSkillMdChanged = () => {
    const folders = vscode.workspace.workspaceFolders;
    if (!folders) return;
    for (const folder of folders) {
      const root = folder.uri.fsPath;
      if (fs.existsSync(path.join(root, "skillet.toml"))) {
        runLint(`workspace:${root}`, root);
        break;
      }
    }
  };
  context.subscriptions.push(skillWatcher.onDidChange(onSkillMdChanged));
  context.subscriptions.push(skillWatcher.onDidCreate(onSkillMdChanged));
  context.subscriptions.push(skillWatcher);

  // Clean up diagnostics when a file is closed
  context.subscriptions.push(
    vscode.workspace.onDidCloseTextDocument((document) => {
      diagnosticsCollection.delete(document.uri);
    })
  );

  context.subscriptions.push(diagnosticsCollection);
  context.subscriptions.push(outputChannel);
  context.subscriptions.push(REF_DECORATION);
}

export function deactivate(): void {
  // Cancel all in-flight processes
  for (const proc of inFlightProcesses.values()) {
    proc.kill();
  }
  inFlightProcesses.clear();

  // Cancel all debounce timers
  for (const timer of debounceTimers.values()) {
    clearTimeout(timer);
  }
  debounceTimers.clear();
}
