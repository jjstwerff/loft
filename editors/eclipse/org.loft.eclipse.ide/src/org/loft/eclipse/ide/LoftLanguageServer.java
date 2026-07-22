// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
package org.loft.eclipse.ide;

import java.util.List;

import org.eclipse.lsp4e.server.ProcessStreamConnectionProvider;

/**
 * Launches the {@code loft-lsp} language server for LSP4E over stdio.
 *
 * <p>The binary is resolved, in order, from the {@code loft.lsp} system property, the
 * {@code LOFT_LSP} environment variable, else {@code loft-lsp} on the {@code PATH}. Build it
 * with {@code cargo build --release --bin loft-lsp} and either put {@code target/release} on
 * your {@code PATH} or set {@code LOFT_LSP} to the absolute path.
 *
 * <p>This class adds no loft-specific behaviour — it only spawns the server. All language
 * intelligence (diagnostics, hover, completion, rename, semantic tokens, …) is computed by
 * {@code loft-lsp} itself and delivered to Eclipse through LSP4E.
 */
public class LoftLanguageServer extends ProcessStreamConnectionProvider {

	public LoftLanguageServer() {
		String cmd = System.getProperty("loft.lsp");
		if (cmd == null || cmd.isBlank()) {
			cmd = System.getenv().getOrDefault("LOFT_LSP", "loft-lsp");
		}
		setCommands(List.of(cmd));
		// A valid CWD is required; the server discovers the real workspace root from the
		// `initialize` request's rootUri, so the user's home is a safe launch directory.
		setWorkingDirectory(System.getProperty("user.home"));
	}
}
