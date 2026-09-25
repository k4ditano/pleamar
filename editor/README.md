# pleamar in an editor

Two things, and both come out of the **compiler's vocabulary**, not out of a
list written apart: if the language changes and this does not, `./run-tests.sh` says so.

## The errors while typing

`pleamar --lsp` is a language server over standard input and output. It gives:

- **the errors with their place**, the same ones as when launching the scene, with
  their "did you mean…?". On save and while typing, and also in the libraries it imports;
- **which words are valid here**: inside a `box`, its properties; inside a
  `path`, its steps; after `anchor:`, the anchors that exist; in an expression, the
  facts and the properties **of this scene**; after `emit`, its events;
- **what the word under the cursor means**, and what it is the name of;
- **going to where a name was declared**, even if it is in an imported library;
- **where that name is used**, and **renaming it** in all those places at once;
- **the outline of the file**: everything it declares, with its kind.

Renaming touches the `.plm` files, not the logic: if a `.luau` writes `fact.volume`,
that has to be changed by hand. The compiler warns all the same, because the old name
no longer exists.

### Neovim

```lua
vim.filetype.add({ extension = { plm = "plm" } })
vim.api.nvim_create_autocmd("FileType", {
  pattern = "plm",
  callback = function(a)
    vim.lsp.start({ name = "pleamar", cmd = { "pleamar", "--lsp" },
                    root_dir = vim.fs.dirname(a.file) }, { bufnr = a.buf })
  end,
})
```

### Helix

In `languages.toml`:

```toml
[[language]]
name = "plm"
scope = "source.plm"
file-types = ["plm"]
comment-token = "//"
language-servers = ["pleamar"]

[language-server.pleamar]
command = "pleamar"
args = ["--lsp"]
```

### VS Code

It needs an extension that launches the server (`vscode-languageclient`), or
any of the generic LSP ones pointing at `pleamar --lsp`.

## The highlighting

- **Vim and Neovim**: `plm.vim` goes in `~/.config/nvim/syntax/plm.vim`.
- **VS Code**: `plm.tmLanguage.json` is the TextMate grammar of an extension
  with `"scopeName": "source.plm"` and `"language": "plm"`.

Both are regenerated with `pleamar --highlight vim` and `pleamar --highlight vscode`.
They are kept here so pleamar does not have to be compiled just to edit a file.
