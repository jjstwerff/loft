-- Copyright (c) 2026 Jurjen Stellingwerff
-- SPDX-License-Identifier: LGPL-3.0-or-later
--
-- Neovim integration for the loft language (@PLN63 IDE.NEOVIM).
--
-- The LSP side is DEPENDENCY-FREE — it uses Neovim's built-in `vim.lsp` (0.8+),
-- so `require('loft').setup()` is all you need to get diagnostics, completion,
-- hover, go-to-definition, references, rename, code actions, inlay hints,
-- semantic tokens, and tracker-tag hovers from `loft-lsp`.  The DAP side (full
-- debugging via `loft-dap`: breakpoints, stepping, variables, data breakpoints,
-- reverse execution) is registered when nvim-dap AND the `loft-dap` binary are both
-- present — see editors/nvim/USAGE.md for the debug keymaps.

local M = {}

-- Resolve a binary: an explicit path override if it is executable, else the bare
-- name if it is on $PATH, else nil.
local function find_bin(name, override)
  if override and override ~= "" and vim.fn.executable(override) == 1 then
    return override
  end
  if vim.fn.executable(name) == 1 then
    return name
  end
  return nil
end

-- The workspace root for a buffer: the nearest ancestor holding `loft.toml` or
-- `.git`, else the cwd.  Uses `vim.fs` (Neovim 0.8+), not `vim.fs.root` (0.10+),
-- so it works on older Neovim too.
local function root_dir(bufnr)
  local name = vim.api.nvim_buf_get_name(bufnr)
  if name == "" then
    return vim.fn.getcwd()
  end
  local found = vim.fs.find({ "loft.toml", ".git" }, {
    upward = true,
    path = vim.fs.dirname(name),
  })[1]
  return found and vim.fs.dirname(found) or vim.fn.getcwd()
end

-- Buffer-local keymaps on attach.  These mirror Neovim 0.11's default LSP maps
-- (harmless duplicates there) and add `gd`; on older Neovim they set them up.
local function on_attach(_, bufnr)
  local function map(lhs, rhs, desc)
    vim.keymap.set("n", lhs, rhs, { buffer = bufnr, silent = true, desc = "loft: " .. desc })
  end
  map("gd", vim.lsp.buf.definition, "go to definition")
  map("K", vim.lsp.buf.hover, "hover")
  map("grn", vim.lsp.buf.rename, "rename")
  map("grr", vim.lsp.buf.references, "find references")
  map("gra", vim.lsp.buf.code_action, "code action")
  map("gO", vim.lsp.buf.document_symbol, "document symbols")
  map("<leader>f", function()
    vim.lsp.buf.format({ async = true })
  end, "format")
  -- Inlay hints (Neovim 0.10+): enable for this buffer if available.
  if vim.lsp.inlay_hint then
    pcall(vim.lsp.inlay_hint.enable, true, { bufnr = bufnr })
  end
  -- Completion: `vim.lsp` sets `omnifunc`, so <C-x><C-o> completes.  On Neovim
  -- 0.11+ turn on autotrigger too (harmless no-op on older versions).
  if vim.lsp.completion and vim.lsp.completion.enable then
    pcall(vim.lsp.completion.enable, true, vim.lsp.get_clients and 0 or nil, bufnr, { autotrigger = true })
  end
end

--- Set up loft LSP (+ DAP when available).
--- @param opts table|nil  { loft_lsp = "<path>", loft_dap = "<path>", stdlib_dir = "<path>" }
function M.setup(opts)
  opts = opts or {}

  -- `.loft` files are loft (idempotent with ftdetect/loft.lua).
  vim.filetype.add({ extension = { loft = "loft" } })

  -- ── loft-lsp — start the language server on each loft buffer ────────────────
  local lsp_cmd = find_bin("loft-lsp", opts.loft_lsp)
  if lsp_cmd then
    local group = vim.api.nvim_create_augroup("loft_lsp", { clear = true })
    vim.api.nvim_create_autocmd("FileType", {
      group = group,
      pattern = "loft",
      callback = function(args)
        vim.lsp.start({
          name = "loft-lsp",
          cmd = { lsp_cmd },
          root_dir = root_dir(args.buf),
          on_attach = on_attach,
        })
      end,
    })
  else
    vim.notify(
      "[loft] `loft-lsp` not found. Build it (`cargo build --release --bin loft-lsp`) "
        .. "and put target/release on $PATH, or call require('loft').setup({ loft_lsp = '/abs/path/to/loft-lsp' }).",
      vim.log.levels.WARN
    )
  end

  -- ── loft-dap — register the debug adapter IF nvim-dap + the binary exist ────
  -- Full DAP surface (D0–D6 + VE/SF/DB/RX); bind the debug keys per editors/nvim/USAGE.md.
  local dap_cmd = find_bin("loft-dap", opts.loft_dap)
  local ok, dap = pcall(require, "dap")
  if ok and dap_cmd then
    dap.adapters.loft = { type = "executable", command = dap_cmd }
    dap.configurations.loft = {
      {
        type = "loft",
        request = "launch",
        name = "Run current file",
        program = "${file}",
        stopOnEntry = false,
      },
    }
  end
end

return M
