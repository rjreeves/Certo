-- Certo LSP configuration for Neovim (requires nvim-lspconfig)
--
-- Prerequisites:
--   - nvim-lspconfig installed
--   - certo-lsp on your PATH  (cargo install --git https://github.com/rjreeves/Certo --bin certo-lsp)
--
-- Usage: add this file to your Neovim config, e.g. require("certo") in init.lua

local lspconfig = require("lspconfig")
local configs   = require("lspconfig.configs")

-- Register certo-lsp if not already known to lspconfig
if not configs.certo_lsp then
    configs.certo_lsp = {
        default_config = {
            cmd          = { "certo-lsp" },
            filetypes    = { "certo" },
            root_dir     = lspconfig.util.root_pattern("certo.toml", ".git"),
            settings     = {},
        },
    }
end

lspconfig.certo_lsp.setup({
    on_attach = function(_, bufnr)
        local opts = { buffer = bufnr, noremap = true, silent = true }
        vim.keymap.set("n", "gd",       vim.lsp.buf.definition,     opts)
        vim.keymap.set("n", "K",        vim.lsp.buf.hover,           opts)
        vim.keymap.set("n", "<C-space>",vim.lsp.buf.completion,      opts)
        vim.keymap.set("n", "[d",       vim.diagnostic.goto_prev,    opts)
        vim.keymap.set("n", "]d",       vim.diagnostic.goto_next,    opts)
    end,
    capabilities = require("cmp_nvim_lsp").default_capabilities(),  -- optional: nvim-cmp
})

-- Filetype detection for .certo and .cto files
vim.filetype.add({
    extension = {
        certo = "certo",
        cto   = "certo",
    },
})
