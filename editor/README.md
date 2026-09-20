# pleamar en un editor

Dos cosas, y las dos salen del **vocabulario del compilador**, no de una lista
escrita aparte: si el lenguaje cambia y esto no, `./probar.sh` lo dice.

## Los fallos mientras escribes

`pleamar --lsp` es un servidor de lenguaje por la entrada y la salida. Da:

- **los fallos con su sitio**, los mismos que al lanzar la escena, con su «did you
  mean…?». Al guardar y al teclear, y también en las bibliotecas que importa;
- **qué palabras valen aquí**: dentro de un `box`, sus propiedades; dentro de un
  `path`, sus pasos; tras `anchor:`, las anclas que existen;
- **qué significa** la palabra bajo el cursor.

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

En `languages.toml`:

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

Necesita una extensión que lance el servidor (`vscode-languageclient`), o
cualquiera de las genéricas de LSP apuntando a `pleamar --lsp`.

## El resaltado

- **Vim y Neovim**: `plm.vim` va en `~/.config/nvim/syntax/plm.vim`.
- **VS Code**: `plm.tmLanguage.json` es la gramática TextMate de una extensión
  con `"scopeName": "source.plm"` y `"language": "plm"`.

Los dos se rehacen con `pleamar --resaltado vim` y `pleamar --resaltado vscode`.
Se guardan aquí para no tener que compilar pleamar solo para editar un fichero.
