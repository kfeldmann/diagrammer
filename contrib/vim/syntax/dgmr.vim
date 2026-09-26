" Vim syntax file
" Language:    diagrammer (dgmr)
" Maintainer:  diagrammer contributors
" Last Change: 26 Sep 2026
" Filenames:   *.dgmr
"
" Matched against the v0 grammar in docs/grammar.md. The language is
" contextual: shapes only follow ':', edge styles only appear inside an
" edge body, directions only follow 'diagram'/'group', and the reserved
" words `diagram`/`group`/`end` are only reserved at statement start (a
" node may legally be named `box`, `thick`, or `end-2`). The highlighting
" below is positional for the same reason.
"
" Everything is single-line -- the grammar is line-oriented and strings
" cannot wrap -- so unterminated constructs never run away across the file.

if exists("b:current_syntax")
  finish
endif

" 'C' in 'cpoptions' would break the \-continuations below.
let s:cpo_save = &cpo
set cpo&vim

syn case match

" ---------------------------------------------------------------------------
" Comments: '#' to end of line, literal inside strings.
" ---------------------------------------------------------------------------
syn keyword dgmrTodo contained TODO FIXME XXX NOTE
syn match dgmrComment /#.*/ contains=dgmrTodo

" ---------------------------------------------------------------------------
" Strings: double-quoted. \" \\ \n are the only escapes the lexer accepts;
" any other \X is a grammar error, so paint it as one. The skip pattern
" treats any \X as an escape so the region still closes on the real quote.
" ---------------------------------------------------------------------------
syn match dgmrStringBadEscape /\\./ contained
syn match dgmrStringEscape /\\\%(["\\n]\)/ contained
syn region dgmrString start=/"/ skip=+\\.+ end=/"/ oneline
    \ contains=dgmrStringEscape,dgmrStringBadEscape

" ---------------------------------------------------------------------------
" Reserved words: `diagram`, `group`, `end` -- statement initial only
" (leading indentation is allowed and not significant). A mid-line
" occurrence sits in a bare-id position where these words are illegal, so
" flag it. Ids may contain hyphens (`end-2` is a plain id, not `end`).
" ---------------------------------------------------------------------------
syn match dgmrReservedErr
    \ /\%(^\s*\|[A-Za-z0-9_]-\?\)\@<!\<\%(diagram\|group\|end\)\>\%(-[A-Za-z0-9_]\)\@!/
syn match dgmrStatement /^\s*\zs\%(diagram\|group\|end\)\>\%(-[A-Za-z0-9_]\)\@!/

" ---------------------------------------------------------------------------
" Directions: only in the slot right after 'diagram' or 'group'.
" ---------------------------------------------------------------------------
syn match dgmrDirection
    \ /\%(^\s*\%(diagram\|group\)\>\%(-[A-Za-z0-9_]\)\@!\s\+\)\@<=\<\%(top-down\|bottom-up\|left-right\|right-left\)\>\%(-[A-Za-z0-9_]\)\@!/

" ---------------------------------------------------------------------------
" Shapes: only after the ':' of a nodespec.
" ---------------------------------------------------------------------------
syn match dgmrShape /:\s*\zs\%(box\|cylinder\)\>\%(-[A-Za-z0-9_]\)\@!/

" ---------------------------------------------------------------------------
" Attributes: `name="value"`, whitespace allowed around '='. The known
" names (nodes color/fill/text, edges color/text/from/to, groups
" color/fill/line/text) highlight as attribute names; any other id before
" an '=' is an unknown attribute and a resolve error, so flag it. (Values
" are plain strings; position-specific validity is the parser's job.)
" ---------------------------------------------------------------------------
syn match dgmrAttrErr /\<[A-Za-z_][A-Za-z0-9_]*\%(-[A-Za-z0-9_]\+\)*\ze\s*=/
syn match dgmrAttrName /\<\%(color\|fill\|text\|line\|from\|to\)\>\ze\s*=/

" ---------------------------------------------------------------------------
" Edges: '--' opens an optional edge body closed by '-->'. Everything
" between the dashes belongs to the edge: an optional style, label, and
" attrs. `A-->B` and `A --> B` parse identically and highlight alike.
" ---------------------------------------------------------------------------
syn match dgmrEdgeStyle /\<\%(solid\|dotted\|dashed\|thick\)\>\%(-[A-Za-z0-9_]\)\@!/ contained
syn match dgmrEdgeOp /--/
syn match dgmrArrow /-->/
" No 'keepend': a quoted label may legally contain '-->' (e.g.
" `A -- "a --> b" --> C`); by default the contained string obscures the
" inner end match and the body runs to the real `-->`.
syn region dgmrEdgeBody matchgroup=dgmrEdgeOp start=/--\%(>\)\@!/ matchgroup=dgmrArrow end=/-->/ oneline
    \ contains=dgmrEdgeStyle,dgmrString,dgmrAttrName,dgmrAttrErr,dgmrReservedErr,dgmrComment

" ---------------------------------------------------------------------------
" Default highlight groups.
" ---------------------------------------------------------------------------
hi def link dgmrComment       Comment
hi def link dgmrTodo          Todo
hi def link dgmrString        String
hi def link dgmrStringEscape  SpecialChar
hi def link dgmrStringBadEscape Error
hi def link dgmrReservedErr   Error
hi def link dgmrStatement     Statement
hi def link dgmrDirection     Constant
hi def link dgmrShape         Type
hi def link dgmrAttrName      PreProc
hi def link dgmrAttrErr       Error
hi def link dgmrEdgeStyle     Type
hi def link dgmrEdgeOp        Operator
hi def link dgmrArrow         Special

let &cpo = s:cpo_save
unlet s:cpo_save

let b:current_syntax = "dgmr"