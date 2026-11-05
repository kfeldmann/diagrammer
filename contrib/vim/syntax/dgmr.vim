" Vim syntax file
" Language:    diagrammer (dgmr)
" Maintainer:  diagrammer contributors
" Last Change: 26 Sep 2026
" Filenames:   *.dgmr
"
" Matched against the v0 grammar in docs/grammar.md. The language is
" contextual: shapes only follow ':', edge styles only appear inside an
" edge body, directions only follow 'diagram'/'group', and the reserved
" words `diagram`/`group`/`end` plus the page-furniture keywords
" `border`/`titleblock`/`textblock` are only reserved at statement start (a
" node may legally be named `box`, `thick`, or `end-2`). The highlighting
" below is positional for the same reason.
"
" Everything is single-line -- the grammar is line-oriented and strings
" cannot wrap -- so unterminated constructs never run away across the file,
" with one deliberate exception: the page-furniture blocks (`titleblock`,
" `textblock`) are multi-line and their regions end at their own `end` line
" (or at the end of the file, if it is missing).

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
" Reserved words, two classes (docs/grammar.md "Reserved words"):
"
"  * `diagram`, `group`, `end` are FULLY reserved as identifiers, so a
"    mid-line occurrence sits in a bare-id position where the word is
"    illegal -- flag it (dgmrReservedErr).
"  * `border`, `titleblock`, `textblock` are reserved ONLY at statement start,
"    because that is the one position where the parser must choose between "a
"    node declaration" and "a block". A node may still be named `border`
"    (`a --> border` is a node), so they must NOT be flagged mid-line -- only
"    statement-initial (dgmrStatement).
"
" Leading indentation is allowed and not significant in both classes, and ids
" may contain hyphens (`end-2` is a plain id, not `end`).
"
" `right`, `top`, and the other anchors are not here either: they are
" contextual, and only mean "anchor" in the slot after `textblock` (see
" dgmrAnchor).
" ---------------------------------------------------------------------------
syn match dgmrReservedErr
    \ /\%(^\s*\|[A-Za-z0-9_]-\?\)\@<!\<\%(diagram\|group\|end\)\>\%(-[A-Za-z0-9_]\)\@!/
syn match dgmrStatement /^\s*\zs\%(diagram\|group\|end\|border\|titleblock\|textblock\)\>\%(-[A-Za-z0-9_]\)\@!/

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
" Text block anchors: the eight compass words, only in the slot right after
" `textblock`. Contextual like directions: `right --> left` is two nodes.
" ---------------------------------------------------------------------------
syn match dgmrAnchor
    \ /\%(^\s*textblock\>\%(-[A-Za-z0-9_]\)\@!\s\+\)\@<=\<\%(top\|upper-right\|right\|lower-right\|bottom\|lower-left\|left\|upper-left\)\>\%(-[A-Za-z0-9_]\)\@!/

" ---------------------------------------------------------------------------
" Page furniture blocks (M21). Both are multi-line, unlike everything else in
" the grammar: the region runs from the statement keyword to its own `end`
" line (a line whose trimmed content is exactly `end`, which is why the end
" pattern anchors `$` -- `end of the line` is prose, not a terminator).
"
" Two things need care inside them:
"
"  * `titleblock` fields are `Key = "value"` with keys that are OPEN by
"    design, so `dgmrAttrErr` (any id before '=') must not fire there -- it
"    would light the whole table red. The region simply does not contain it;
"    `dgmrTitleKey` claims every key, and `dgmrAttrName` (defined later, so it
"    wins the tie) claims the two style keys `color`/`fill`.
"  * A `textblock` body is RAW prose: `#` is not a comment, `-->` is not an
"    arrow, and Markdown markers are not operators. `dgmrTextBody` is
"    anchored with `^` so it can only match a *whole* body line, never the
"    tail of the header line -- which is what lets the anchor and attrs on the
"    `textblock` line still highlight.
" ---------------------------------------------------------------------------
syn region dgmrTitleBlock
    \ matchgroup=dgmrStatement start=/^\s*\zs\%(titleblock\)\>\%(-[A-Za-z0-9_]\)\@!/
    \ matchgroup=dgmrStatement end=/^\s*\zs\%(end\)\>\%(-[A-Za-z0-9_]\)\@!\s*$/
    \ contains=dgmrString,dgmrStringEscape,dgmrStringBadEscape,dgmrTitleKey,dgmrAttrName,dgmrComment,dgmrTodo,dgmrReservedErr
    \ keepend
" A title-block key: an id before '=' that is not a style key. The style keys
" are claimed by `dgmrAttrName`, which is defined later and therefore wins.
syn match dgmrTitleKey /\<[A-Za-z_][A-Za-z0-9_]*\%(-[A-Za-z0-9_]\+\)*\ze\s*=/ contained

syn region dgmrTextBlock
    \ matchgroup=dgmrStatement start=/^\s*\zs\%(textblock\)\>\%(-[A-Za-z0-9_]\)\@!/
    \ matchgroup=dgmrStatement end=/^\s*\zs\%(end\)\>\%(-[A-Za-z0-9_]\)\@!\s*$/
    \ contains=dgmrTextBody,dgmrAnchor,dgmrAttrName,dgmrAttrErr,dgmrString,dgmrStringEscape,dgmrStringBadEscape
    \ keepend
" A whole body line, verbatim. `^`-anchored so the header line's tail is never
" swallowed; `.` cannot cross the newline, so a body line stays one line.
" Deliberately UNLINKED: prose is prose, so the point of this match is to
" *block* the comment/arrow/operator/string rules from firing, not to paint
" the body. (The Markdown subset is content, not syntax -- it renders, it is
" not highlighted.)
syn match dgmrTextBody /^.*$/ contained

" ---------------------------------------------------------------------------
" Attributes: `name="value"`, whitespace allowed around '='. The known
" names (nodes color/fill/text, edges color/text/from/to, groups
" color/fill/line/text, border color/line, textblock color/fill/text/width)
" highlight as attribute names; any other id before an '=' is an unknown
" attribute and a resolve error, so flag it. (Values are plain strings;
" position-specific validity is the parser's job.)
"
" A title block is the one place where this does NOT apply -- its keys are
" open by design -- so its region does not contain dgmrAttrErr at all.
" ---------------------------------------------------------------------------
syn match dgmrAttrErr /\<[A-Za-z_][A-Za-z0-9_]*\%(-[A-Za-z0-9_]\+\)*\ze\s*=/
syn match dgmrAttrName /\<\%(color\|fill\|text\|line\|from\|to\|width\)\>\ze\s*=/

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
hi def link dgmrAnchor        Constant
hi def link dgmrShape         Type
hi def link dgmrAttrName      PreProc
hi def link dgmrAttrErr       Error
hi def link dgmrEdgeStyle     Type
hi def link dgmrEdgeOp        Operator
hi def link dgmrArrow         Special
hi def link dgmrTitleKey      PreProc

let &cpo = s:cpo_save
unlet s:cpo_save

let b:current_syntax = "dgmr"