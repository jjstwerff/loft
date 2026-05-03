// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// VS Code extension entry point.  Registers two commands that make
// running .loft files feel native:
//
//   loft.runFile        — `loft <file>` in an integrated terminal
//   loft.runFileNative  — `loft --native <file>` in an integrated terminal
//
// Both appear as play-buttons in the editor title bar when a .loft
// file is active, and as default keybindings (F5 for run, Ctrl+F5 for
// run-native) gated to .loft files so they don't clash with other
// languages' debug shortcuts.
//
// The terminal is launched with `shellPath` + `shellArgs` so VS Code
// executes the loft binary directly via `execve` — no shell parsing,
// no quoting concerns.  The trade-off is that each run replaces the
// previous "Loft" terminal (dispose + recreate); cross-run scroll-
// back is lost, but per-run scroll within the active terminal works
// normally.  This avoids the shell-injection class of bugs entirely
// (CodeQL `js/incomplete-sanitization` flagged the prior `sendText`
// + manual quote approach for not escaping backslashes on Windows).

import * as vscode from "vscode";

/// Build the path argument for the `loft` invocation.  Uses the
/// active editor's URI; if no editor is active or the file is
/// unsaved, fail with a clear message rather than running with
/// garbage input.
function activeLoftFile(): string | undefined {
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    vscode.window.showErrorMessage("Loft: no active file to run.");
    return undefined;
  }
  if (editor.document.isUntitled) {
    vscode.window.showErrorMessage("Loft: save the file before running.");
    return undefined;
  }
  if (editor.document.languageId !== "loft") {
    vscode.window.showErrorMessage(
      "Loft: active file is not a .loft file (language id: " +
        editor.document.languageId +
        ")."
    );
    return undefined;
  }
  return editor.document.uri.fsPath;
}

/// Read the user's preferred `loft` binary path from settings, or
/// fall back to `loft` (resolved via $PATH).  Lets users with
/// multiple installs (e.g. cargo install vs Homebrew vs a local
/// dev build) pin a specific binary without overriding $PATH.
function loftBinary(): string {
  const cfg = vscode.workspace.getConfiguration("loft");
  return cfg.get<string>("binaryPath", "loft");
}

/// Save the active file (if dirty) before invoking `loft` on it.
/// Mirrors what every other language's "Run" button does — running
/// stale on-disk content while the editor shows newer content
/// would be deeply confusing.
async function saveAndRun(extraArgs: string[]): Promise<void> {
  const file = activeLoftFile();
  if (!file) {
    return;
  }
  const editor = vscode.window.activeTextEditor!;
  if (editor.document.isDirty) {
    await editor.document.save();
  }

  // Dispose any existing Loft terminal so we don't accumulate
  // stale tabs across runs.  Each run gets a fresh terminal
  // launched directly via `execve(loftBin, args)`; no shell
  // parsing, no quoting, no injection class of bugs.
  for (const term of vscode.window.terminals) {
    if (term.name === "Loft") {
      term.dispose();
    }
  }
  const term = vscode.window.createTerminal({
    name: "Loft",
    shellPath: loftBinary(),
    shellArgs: [...extraArgs, file],
  });
  term.show(true);
}

export function activate(context: vscode.ExtensionContext) {
  context.subscriptions.push(
    vscode.commands.registerCommand("loft.runFile", () => saveAndRun([])),
    vscode.commands.registerCommand("loft.runFileNative", () =>
      saveAndRun(["--native"])
    )
  );
}

export function deactivate() {
  // No-op; VS Code disposes registered commands automatically.
}
