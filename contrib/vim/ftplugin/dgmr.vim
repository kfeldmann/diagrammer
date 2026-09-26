" Vim filetype plugin for diagrammer (.dgmr) files.

if exists("b:did_ftplugin")
  finish
endif
let b:did_ftplugin = 1

" '#' starts a comment outside strings. 'iskeyword' is deliberately left
" alone: ids may contain hyphens but the lexer only joins '-' when it is
" followed by a word char, and syntax patterns rely on the default
" word-boundary behaviour (e.g. `A--dotted-->B` must see `dotted` as a
" word).
setlocal comments=b:#
setlocal commentstring=#\ %s
setlocal suffixesadd=.dgmr

let b:undo_ftplugin = "setlocal comments< commentstring< suffixesadd<"