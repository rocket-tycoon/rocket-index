; Indentation rules for F# in Zed
; These rules define when to increase or decrease indentation

; Indent after let bindings with = and no body on same line
(function_or_value_defn
  "=" @indent)

; Indent after type definitions
(type_definition
  "=" @indent)

; Indent after match/with
(match_expression
  "with" @indent)

; Indent after if/then
(if_expression
  "then" @indent)

; Indent after else
(if_expression
  "else" @indent)

; Indent after elif
(if_expression
  "elif" @indent)

; Indent after fun ->
(fun_expression
  "->" @indent)

; Indent after function |
(function_expression
  "|" @indent)

; Indent after try
(try_expression
  "try" @indent)

; Indent after with in try/with
(try_expression
  "with" @indent)

; Indent after finally
(try_expression
  "finally" @indent)

; Indent after for/do
(for_expression
  "do" @indent)

; Indent after while/do
(while_expression
  "do" @indent)

; Indent after module definition
(module_defn
  "=" @indent)

; Indent after class type definition
(class_type_defn) @indent

; Indent after interface definition
(interface_type_defn) @indent

; Indent after record type definition opening brace
(record_type_defn
  "{" @indent)

; Indent inside blocks
(begin_end_expression
  "begin" @indent)

; Dedent on closing constructs
"end" @outdent
"}" @outdent
"|]" @outdent
"]" @outdent
")" @outdent

; Indent continuation for long expressions
(infix_expression) @indent

; Indent after do keyword in computation expressions
(ce_expression
  "do" @indent)

; Indent after yield
(yield_expression) @indent

; Indent after return
(return_expression) @indent
