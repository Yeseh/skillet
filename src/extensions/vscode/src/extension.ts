import * as vscode from "vscode";
import * as cp from "child_process";
import * as path from "path";
import * as fs from "fs";
import { LanguageClient, TransportKind } from "vscode-languageclient/node";
import type {
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";

// ── Types ─────────────────────────────────────────────────────────────────────

interface LintRelatedLocation {
  path: string;
  line?: number;
  col?: number;
  message: string;
}

interface LintDiagnostic {
  rule: string;
  severity: "error" | "warning" | "info";
  skill: string;
  message: string;
  path?: string;
  line?: number;
  col?: number;
  related?: LintRelatedLocation[];
}

// ── Module-level state ────────────────────────────────────────────────────────

let outputChannel: vscode.OutputChannel;
let diagnosticsCollection: vscode.DiagnosticCollection;
let languageClient: LanguageClient | undefined;

/** Whether we've already shown the "skillet not found" warning this session. */
let cliMissingWarned = false;

/** Per-document debounce timers. */
const debounceTimers = new Map<string, ReturnType<typeof setTimeout>>();

/** Per-document in-flight lint processes (for cancellation). */
const inFlightProcesses = new Map<string, cp.ChildProcess>();

// ── Typed-ref decoration ──────────────────────────────────────────────────────

function createRefDecorationType(): vscode.TextEditorDecorationType {
  return vscode.window.createTextEditorDecorationType({
    backgroundColor: new vscode.ThemeColor("badge.background"),
    color: new vscode.ThemeColor("badge.foreground"),
    borderRadius: "3px",
    // Small padding improves pill shape readability
    before: { margin: "0 1px" },
    after: { margin: "0 1px" },
  });
}

const TYPED_REF_RE =
  /((?<!`)`(?!`))(ref|cmd|skill|var|env|agent|path|url)(::)([^`]*?)((?<!`)`(?!`))/g;

function applyRefDecorations(
  editor: vscode.TextEditor,
  decorationType: vscode.TextEditorDecorationType
): void {
  const ranges: vscode.Range[] = [];
  const text = editor.document.getText();
  let match: RegExpExecArray | null;
  TYPED_REF_RE.lastIndex = 0;
  while ((match = TYPED_REF_RE.exec(text)) !== null) {
    const start = editor.document.positionAt(match.index);
    const end = editor.document.positionAt(match.index + match[0].length);
    ranges.push(new vscode.Range(start, end));
  }
  editor.setDecorations(decorationType, ranges);
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

function executablePath(workspaceRoot?: string): string {
  const cfg = vscode.workspace.getConfiguration("skillet");
  const raw = cfg.get<string>("executablePath", "skillet");
  if (path.isAbsolute(raw)) {
    return raw;
  }
  // Relative path: resolve against workspace root
  if (raw !== "skillet" && raw !== "") {
    return workspaceRoot ? path.resolve(workspaceRoot, raw) : raw;
  }
  return raw;
}

function isPedantic(): boolean {
  return vscode.workspace
    .getConfiguration("skillet")
    .get<boolean>("pedantic", false);
}

/** Plugin source directories extracted from `skillet.toml`. */
interface WorkspaceDirs {
  /** Plugin `src_dir` values (relative to workspace root). */
  srcDirs: string[];
}

/**
 * Minimal `skillet.toml` reader. Extracts plugin `src_dir` values to map
 * skill diagnostics to source files. Handles the well-formed TOML that
 * `skillet` emits; it is not a general-purpose parser.
 */
function parseWorkspaceDirs(root: string): WorkspaceDirs {
  const srcDirs: string[] = [];

  let raw: string;
  try {
    raw = fs.readFileSync(path.join(root, "skillet.toml"), "utf8");
  } catch {
    return { srcDirs };
  }

  let section = "";
  for (const line of raw.split(/\r?\n/)) {
    const trimmed = line.replace(/#.*$/, "").trim();
    if (trimmed === "") {
      continue;
    }
    const header = trimmed.match(/^\[(.+)\]$/);
    if (header) {
      section = header[1].trim();
      continue;
    }
    const kv = trimmed.match(/^([A-Za-z0-9_]+)\s*=\s*(.+)$/);
    if (!kv) {
      continue;
    }
    const key = kv[1];
    const value = kv[2].trim().replace(/^["']|["']$/g, "");
    const isPlugin = section.startsWith("plugin.");
    if (key === "src_dir" && (isPlugin || section === "workspace")) {
      srcDirs.push(value);
    }
  }

  return { srcDirs };
}

/** Returns the first path in `candidates` that exists on disk, or undefined. */
function firstExisting(candidates: string[]): string | undefined {
  return candidates.find((p) => fs.existsSync(p));
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

function resolveDiagnosticPath(
  d: LintDiagnostic,
  workspaceRoot: string,
  dirs: WorkspaceDirs
): string {
  if (d.path) {
    return path.isAbsolute(d.path)
      ? d.path
      : path.join(workspaceRoot, d.path);
  }

  if (d.skill && d.skill !== "<workspace>") {
    const fallback = firstExisting([
      ...dirs.srcDirs.map((src) =>
        path.join(workspaceRoot, src, d.skill, `${d.skill}.pan`)
      ),
      path.join(workspaceRoot, "agents", `${d.skill}.agent.pan`),
    ]);
    if (fallback) {
      return fallback;
    }
  }

  return path.join(workspaceRoot, "skillet.toml");
}

function toVscodeRange(line?: number, col?: number): vscode.Range {
  // line and col are 1-based from skillet; VS Code uses 0-based
  const zeroLine = line !== undefined ? Math.max(0, line - 1) : 0;
  const startCol = col !== undefined ? Math.max(0, col - 1) : 0;
  return new vscode.Range(zeroLine, startCol, zeroLine, Number.MAX_SAFE_INTEGER);
}

function publishDiagnostics(
  lintDiags: LintDiagnostic[],
  workspaceRoot: string
): void {
  const dirs = parseWorkspaceDirs(workspaceRoot);
  const byUri = new Map<string, vscode.Diagnostic[]>();

  for (const d of lintDiags) {
    const filePath = resolveDiagnosticPath(d, workspaceRoot, dirs);
    const uriStr = vscode.Uri.file(filePath).toString();

    const vscodeDiag = toVscodeDiagnostic(d, workspaceRoot);
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

function toVscodeDiagnostic(
  d: LintDiagnostic,
  workspaceRoot: string
): vscode.Diagnostic {
  const severity =
    d.severity === "error"
      ? vscode.DiagnosticSeverity.Error
      : d.severity === "warning"
      ? vscode.DiagnosticSeverity.Warning
      : vscode.DiagnosticSeverity.Information;

  const diag = new vscode.Diagnostic(
    toVscodeRange(d.line, d.col),
    `${d.message} [${d.rule}]`,
    severity
  );
  diag.source = "skillet";
  diag.code = d.rule;

  if (d.related && d.related.length > 0) {
    diag.relatedInformation = d.related.map((r) => {
      const relPath = path.isAbsolute(r.path)
        ? r.path
        : path.join(workspaceRoot, r.path);
      return new vscode.DiagnosticRelatedInformation(
        new vscode.Location(vscode.Uri.file(relPath), toVscodeRange(r.line, r.col)),
        r.message
      );
    });
  }

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

function startLanguageClient(context: vscode.ExtensionContext): void {
  const workspaceRoot = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  const exe = executablePath(workspaceRoot);
  const serverOptions: ServerOptions = {
    command: exe,
    args: ["lsp"],
    transport: TransportKind.stdio,
  };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "pan" }],
    outputChannel,
  };

  languageClient = new LanguageClient(
    "skillet",
    "Skillet Language Server",
    serverOptions,
    clientOptions
  );
  context.subscriptions.push(languageClient);

  void languageClient
    .start()
    .catch((error: NodeJS.ErrnoException) => {
      if (error.code === "ENOENT") {
        warnCliMissing(exe);
        return;
      }
      outputChannel.appendLine(
        `[error] Failed to start '${exe} lsp': ${error.message}`
      );
      outputChannel.show(true);
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
  diagnosticsCollection =
    vscode.languages.createDiagnosticCollection("skillet");
  startLanguageClient(context);
  const refDecoration = createRefDecorationType();

  // Decorate currently open .pan editors on activation
  for (const editor of vscode.window.visibleTextEditors) {
    if (editor.document.languageId === "pan") {
      applyRefDecorations(editor, refDecoration);
    }
  }

  // Decorate all visible .pan editors when the visible editor set changes (e.g. split views)
  context.subscriptions.push(
    vscode.window.onDidChangeVisibleTextEditors((editors) => {
      for (const editor of editors) {
        if (editor.document.languageId === "pan") {
          applyRefDecorations(editor, refDecoration);
        }
      }
    })
  );

  // Decorate on editor switch
  context.subscriptions.push(
    vscode.window.onDidChangeActiveTextEditor((editor) => {
      if (editor && editor.document.languageId === "pan") {
        applyRefDecorations(editor, refDecoration);
      }
    })
  );

  // Re-decorate on document change (typing) across all visible editors showing it
  context.subscriptions.push(
    vscode.workspace.onDidChangeTextDocument((event) => {
      if (event.document.languageId !== "pan") {
        return;
      }
      for (const editor of vscode.window.visibleTextEditors) {
        if (editor.document === event.document) {
          applyRefDecorations(editor, refDecoration);
        }
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
  context.subscriptions.push(refDecoration);
}

export async function deactivate(): Promise<void> {
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
  await languageClient?.stop();
}
