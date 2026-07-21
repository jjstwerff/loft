-- Copyright (c) 2026 Jurjen Stellingwerff
-- SPDX-License-Identifier: LGPL-3.0-or-later
--
-- Register the `.loft` filetype on startup (runs automatically when this plugin
-- is on the runtimepath), so syntax + the `loft-lsp` FileType autocmd fire even
-- before `require('loft').setup()` is called.
vim.filetype.add({ extension = { loft = "loft" } })
