# VS Code support for diagrammer

VS Code extension providing syntax highlighting for `.dgmr` diagram sources.

- [`syntaxes/dgmr.tmLanguage.json`](syntaxes/dgmr.tmLanguage.json) — TextMate
  grammar matched against [`docs/grammar.md`](../../docs/grammar.md) (v0),
  mirroring [`contrib/vim/syntax/dgmr.vim`](../vim/syntax/dgmr.vim),
- [`language-configuration.json`](language-configuration.json) — `#` line
  comments, quote/paren auto-closing, and a `wordPattern` matching the id
  grammar (`api-server` double-click-selects whole, `end-2` too).

## Install

This directory is a self-contained extension (a `package.json` at its root),
so a symlink or copy into the extensions directory is all it takes:

```sh
# Linux / macOS
ln -s "$PWD" ~/.vscode/extensions/dgmr-vscode

# Windows (admin shell)
mklink /D "%USERPROFILE%\.vscode\extensions\dgmr-vscode" "%CD%"
```

Then restart VS Code (or *Developer: Reload Window*).

To package it as a `.vsix` for distribution:

```sh
npx @vscode/vsce package
```

## What gets highlighted

Same coverage as the vim syntax file (scope names follow TextMate
conventions, so standard themes color them):

- `#` comments (with `TODO`/`FIXME`/`XXX`/`NOTE` called out),
- strings and their `\"`, `\\`, `\n` escapes — any other escape is a
  grammar error and is flagged as one,
- the reserved words `diagram`, `group`, `end` at statement start (`keyword`);
  a mid-line occurrence sits where a bare id is allowed and the parser
  rejects it, so it is flagged (`invalid.illegal`),
- the page-furniture keywords `border`, `titleblock`, `textblock` at
  statement start only (`keyword`) — reserved *only* there, so a node may
  legally be named `border` (`a --> border`) and is not flagged,
- text-block anchors (`top`, `upper-right`, …) only in the slot right
  after `textblock` (`constant.language.anchor`),
- directions (`top-down`, …) only in the slot after `diagram`/`group`
  (`constant.language`),
- shapes (`box`, `cylinder`) only after `:` (`storage.type`),
- edge operators `--` / `-->`, and edge-body styles (`solid`, `dotted`,
  `dashed`, `thick`) only inside an edge body,
- attribute names before `=`: known names (`color`, `fill`, `text`,
  `line`, `from`, `to`, `width`) highlight as attributes
  (`entity.other.attribute-name`), unknown ones are flagged — **except
  inside a `titleblock` body**, where keys are open by design and every key
  highlights as an attribute name,
- `titleblock` / `textblock` bodies: the `titleblock` region highlights
  fields and `color`/`fill` as style keys; the `textblock` region is left
  **unpainted** on purpose, because its body is raw prose where `#` is not
  a comment, `-->` is not an arrow, and Markdown markers are not operators.
  Both regions end at their own `end` line (a line whose trimmed content is
  exactly `end`).

Highlighting is contextual on purpose — the grammar reserves only
`diagram`/`group`/`end` as identifiers (plus the page-furniture keywords at
statement start), so a node may legally be named `box`, `thick`, or
`border` and stays plain. Invalid positions the regexes cannot judge (wrong
attribute for the position, unknown `from`/`to` side, bad enum value) are
left to `diagrammer`'s parser.

## Requirements

VS Code 1.63+ (plain TextMate grammar, no extension host code).
