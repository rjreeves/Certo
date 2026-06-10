# Editor Support

Certo ships a language server (`certo-lsp`) that provides:

| Feature | Supported |
|---------|-----------|
| Diagnostics (parse + type errors) | ✓ |
| Hover (type signatures) | ✓ |
| Go-to-definition | ✓ |
| Completion (keywords + symbols) | ✓ |
| Syntax highlighting | ✓ (VS Code) |

---

## Prerequisites

`certo-lsp` must be on your PATH:

```sh
cargo install --git https://github.com/rjreeves/Certo --bin certo-lsp
```

Or download from the [Releases page](https://github.com/rjreeves/Certo/releases) and place it on your PATH.

---

## VS Code

### Install from VSIX (recommended)

1. Build the extension:
   ```sh
   cd editors/vscode
   npm install
   npx vsce package      # produces certo-language-0.1.0.vsix
   ```
2. In VS Code: **Extensions → ⋯ → Install from VSIX…** → select the `.vsix`

### Install manually (sideload)

Copy the `editors/vscode/` directory to your VS Code extensions folder:

| Platform | Path |
|----------|------|
| Windows  | `%USERPROFILE%\.vscode\extensions\certo-language` |
| macOS    | `~/.vscode/extensions/certo-language` |
| Linux    | `~/.vscode/extensions/certo-language` |

Then reload VS Code.

### Configuration

In `settings.json`:
```json
{
  "certo.serverPath": "certo-lsp"
}
```

Set `"certo.serverPath"` to an absolute path if `certo-lsp` is not on your PATH.

---

## Neovim

Requires [nvim-lspconfig](https://github.com/neovim/nvim-lspconfig).

Add to your `init.lua`:

```lua
require("certo")   -- if editors/neovim/certo.lua is on your runtimepath
```

Or paste the contents of `editors/neovim/certo.lua` directly into your config.

The LSP will activate automatically for any file with a `.cto` or `.cto` extension.

---

## Other editors

Any editor with LSP support can use `certo-lsp`. The server communicates over stdio.

Generic LSP client config:
- **Command:** `certo-lsp`
- **Transport:** stdio
- **File types:** `.cto`, `.cto`
- **Root pattern:** `certo.toml`
