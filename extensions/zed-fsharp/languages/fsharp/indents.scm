; Minimal indentation rules for F#
; Note: Only use patterns that match the actual grammar structure

; Control flow indent/dedent pairs
(match_expression
  "with" @indent)

(if_expression
  "then" @indent)

(if_expression
  "else" @indent)

(try_expression
  "with" @indent)

(try_expression
  "finally" @indent)

; Brackets and block delimiters
(_ "{" @indent)
(_ "[" @indent)
(_ "(" @indent)
(_ "[|" @indent)
(_ "begin" @indent)

(_ "}" @outdent)
(_ "]" @outdent)
(_ ")" @outdent)
(_ "|]" @outdent)
(_ "end" @outdent)
