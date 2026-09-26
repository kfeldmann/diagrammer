# Vim support for diagrammer

Vim/Neovim runtime files for `.dgmr` diagram sources:

- [`syntax/dgmr.vim`](syntax/dgmr.vim) — syntax highlighting matched against
  [`docs/grammar.md`](../../docs/grammar.md) (v0),
- [`ftdetect/dgmr.vim`](ftdetect/dgmr.vim) — filetype detection for `*.dgmr`,
- [`ftplugin/dgmr.vim`](ftplugin/dgmr.vim) — filetype settings (`#` comments,
  `suffixesadd`).

## Install

This directory is a self-contained vim package (it has `ftdetect/`, `syntax/`,
and `ftplugin/` at its root), so a symlink is all it takes.

**Vim 8+ / Neovim packages** (no plugin manager):

```sh
# Vim
ln -s "$PWD" ~/.vim/pack/plugins/start/dgmr

# Neovim
ln -s "$PWD" ~/.config/nvim/pack/plugins/start/dgmr
```

**Pathogen:**

```sh
ln -s "$PWD" ~/.vim/bundle/dgmr
```

**Manual** (no package directories):

```sh
mkdir -p ~/.vim/{ftdetect,syntax,ftplugin}
ln -s "$PWD"/ftdetect/dgmr.vim ~/.vim/ftdetect/dgmr.vim
ln -s "$PWD"/syntax/dgmr.vim   ~/.vim/syntax/dgmr.vim
ln -s "$PWD"/ftplugin/dgmr.vim ~/.vim/ftplugin/dgmr.vim
```

Run from this `contrib/vim` directory (or adjust `$PWD` accordingly).

## What gets highlighted

- `#` comments (with `TODO`/`FIXME`/`XXX`/`NOTE` called out),
- strings and their `\"`, `\\`, `\n` escapes — any other escape is a
  grammar error and is flagged as one,
- the reserved words `diagram`, `group`, `end` at statement start; a
  mid-line occurrence sits where a bare id is allowed and the parser
  rejects it, so it is flagged too,
- directions (`top-down`, …) only in the slot after `diagram`/`group`,
- shapes (`box`, `cylinder`) only after `:`,
- edge operators `--` / `-->`, and edge-body styles (`solid`, `dotted`,
  `dashed`, `thick`) only inside an edge body,
- attribute names before `=`: known names (`color`, `fill`, `text`,
  `line`, `from`, `to`) highlight as attributes, unknown ones are flagged.

Highlighting is contextual on purpose — the grammar reserves only
`diagram`/`group`/`end`, so a node may legally be named `box` or `thick`
and stays plain. Invalid positions the regexes cannot judge (wrong
attribute for the position, unknown `from`/`to` side, bad enum value) are
left to `diagrammer`'s parser.

## Requirements

Vim 7.4+ or Neovim (uses standard `\%()`, `\@<=`, `\@<!` syntax patterns).