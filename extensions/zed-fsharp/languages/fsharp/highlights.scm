; Keywords
[
  "let"
  "rec"
  "and"
  "in"
  "if"
  "then"
  "else"
  "elif"
  "match"
  "with"
  "when"
  "for"
  "to"
  "downto"
  "do"
  "done"
  "while"
  "try"
  "finally"
  "raise"
  "fun"
  "function"
  "type"
  "of"
  "as"
  "module"
  "namespace"
  "open"
  "val"
  "mutable"
  "inline"
  "static"
  "member"
  "override"
  "abstract"
  "default"
  "public"
  "private"
  "internal"
  "new"
  "inherit"
  "interface"
  "class"
  "struct"
  "enum"
  "delegate"
  "async"
  "lazy"
  "yield"
  "yield!"
  "return"
  "return!"
  "use"
  "use!"
  "begin"
  "end"
  "extern"
  "void"
  "upcast"
  "downcast"
  "not"
  "or"
  "mod"
] @keyword

; Boolean literals
[
  "true"
  "false"
] @boolean

; Null literal
"null" @constant.builtin

; Operators
[
  "|>"
  "<|"
  ">>"
  "<<"
  "||"
  "&&"
  "="
  "<>"
  "<"
  ">"
  "<="
  ">="
  "+"
  "-"
  "*"
  "/"
  "%"
  "**"
  "::"
  "@"
  "^"
  "|"
  "&"
  "~~~"
  ">>>"
  "<<<"
  "->"
  "<-"
  ":>"
  ":?>"
  ":?"
] @operator

; Punctuation - Brackets
[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
  "[|"
  "|]"
  "[<"
  ">]"
] @punctuation.bracket

; Punctuation - Delimiters
[
  ","
  ";"
  ":"
  "."
] @punctuation.delimiter

; Pipe in pattern matching
"|" @punctuation.delimiter

; Type annotations
(type_name) @type
(type_argument) @type

; Generic type parameters
(type_argument_defn) @type

; Function definitions
(function_or_value_defn
  (function_declaration_left
    (identifier) @function))

; Value bindings (non-function let bindings)
(function_or_value_defn
  (value_declaration_left
    (identifier_pattern
      (long_identifier
        (identifier) @variable))))

; Function calls
(application_expression
  (long_identifier_or_op
    (long_identifier) @function.call))

; Method calls
(dot_expression
  (long_identifier_or_op
    (long_identifier) @function.method))

; Parameters in function definitions
(argument_patterns
  (long_identifier
    (identifier) @variable.parameter))

; Simple pattern parameter
(argument_patterns
  (identifier_pattern
    (long_identifier
      (identifier) @variable.parameter)))

; Record field definitions
(record_field
  (identifier) @property)

; Record field access
(field_expression
  (long_identifier
    (identifier) @property))

; Union case definitions
(union_type_case
  (identifier) @constructor)

; Union case usage in patterns
(identifier_pattern
  (long_identifier
    (identifier) @constructor))

; Module definitions
(module_defn
  (long_identifier) @namespace)

; Namespace definitions
(namespace
  (long_identifier) @namespace)

; Open statements (imports)
(open_statement
  (long_identifier) @namespace)

; Identifiers (general)
(identifier) @variable
(long_identifier) @variable

; Literals - Strings
(string) @string
(verbatim_string) @string
(triple_quoted_string) @string
(interpolated_string) @string

; Literals - Characters
(char) @character

; Literals - Numbers
(int) @number
(int16) @number
(int32) @number
(int64) @number
(uint16) @number
(uint32) @number
(uint64) @number
(float) @number
(decimal) @number

; Comments
(comment) @comment
(block_comment) @comment

; XML documentation comments
(xml_doc) @comment.documentation

; Attributes
(attribute) @attribute
(attribute_set) @attribute

; Compiler directives
(preproc_if) @keyword.directive
(preproc_else) @keyword.directive
(preproc_endif) @keyword.directive
(preproc_line) @keyword.directive

; Active patterns
(active_pattern_op) @function.special

; Computation expressions
(ce_expression
  (identifier) @function.builtin)

; Special identifiers
((identifier) @variable.builtin
  (#match? @variable.builtin "^(this|base|self)$"))

; Exception handling
"try" @keyword.exception
"with" @keyword.exception
"finally" @keyword.exception
"raise" @keyword.exception
"failwith" @function.builtin
"failwithf" @function.builtin

; Measure types
(measure_type) @type.qualifier
