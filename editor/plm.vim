" Sintaxis de pleamar para Vim y Neovim. La hace `pleamar --resaltado vim`:
" no se escribe a mano, y así no se queda atrás del lenguaje.
if exists("b:current_syntax") | finish | endif
syn keyword plmStatement surface permissions model service spring prop pose fact event text image measure let zone body ellipse box arc line path input clip group popup component children repeat for row column space between layer on every blink wave spin follow look gesture posture
syn keyword plmStatement scene library import language
syn keyword plmKeyword in max while for after from until at by reach within rest inset right middle as via strict each all
syn keyword plmFunction min max abs floor ceil clamp smooth mix if vel
syn keyword plmFunction upper lower
syn keyword plmTrigger press release scroll drag hold enter leave hover away idle key submit focus blur drop change
syn keyword plmEffect toggle emit impulse play focus blur
syn keyword plmStep move line curve close
syn keyword plmConstant true false lively calm quick slow gentle pose linear in_quad out_quad in_cubic out_cubic in_out_sine out_back text number bool image
syn match plmProperty "\<\w\+\ze\s*:"
syn match plmNumber "\<\d\+\(\.\d\+\)\?\(px\|%\|deg\|ms\|s\)\?\>"
syn match plmColour "#[0-9a-fA-F]\{3,8}\>"
syn match plmSpring "\~\w\+"
syn region plmString start=/"/ skip=/\\"/ end=/"/ contains=plmHole
syn region plmHole start=/{/ end=/}/ contained
syn match plmComment "//.*$"
hi def link plmStatement Statement
hi def link plmKeyword Keyword
hi def link plmFunction Function
hi def link plmTrigger Special
hi def link plmEffect Special
hi def link plmStep Type
hi def link plmConstant Constant
hi def link plmProperty Identifier
hi def link plmNumber Number
hi def link plmColour Constant
hi def link plmSpring PreProc
hi def link plmString String
hi def link plmHole SpecialChar
hi def link plmComment Comment
let b:current_syntax = "plm"
