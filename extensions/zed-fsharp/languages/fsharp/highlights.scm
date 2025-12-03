; ----------------------------------------------------------------------------
; Literals and comments

[
  (line_comment)
  (block_comment)
] @comment

; XML doc comments (///) - handled via line_comment matching
((line_comment) @comment.documentation
 (#match? @comment.documentation "^///"))

(const) @constant

; Unit constant
(const
  (unit) @constant.builtin)

; General identifier fallback
(identifier) @variable

; ----------------------------------------------------------------------------
; Types

(type_name type_name: (_) @type.definition)
(exception_definition exception_name: (_) @type.definition)

[
  (_type)
  (atomic_type)
] @type

(union_type_case) @type

; ----------------------------------------------------------------------------
; Modules and namespaces

(fsi_directive_decl . (string) @module)
(import_decl . (_) @module)
(named_module name: (_) @module)
(namespace name: (_) @module)
(module_defn . (_) @module)

; Long identifiers - prefix parts are modules
(long_identifier
  ((identifier)* @module)
  .
  ((identifier)))

; ----------------------------------------------------------------------------
; Functions and members

(function_declaration_left
  . (_) @function
  [
    (argument_patterns)
    (argument_patterns (long_identifier (identifier)))
  ] @variable.parameter)

(member_defn
  (method_or_prop_defn
    [
      (property_or_ident) @function
      (property_or_ident
        instance: (identifier) @variable.parameter.builtin
        method: (identifier) @function.method)
    ]
    args: (_)? @variable.parameter))

(member_signature
  .
  (identifier) @function.member
  (curried_spec
    (arguments_spec
      "*"* @operator
      (argument_spec
        (argument_name_spec
          "?"? @character.special
          name: (_) @variable.parameter)
        (_) @type))))

; Function calls
(application_expression
  .
  [
    (_) @function.call
    (long_identifier_or_op (long_identifier (identifier) (identifier) @function.call))
    (typed_expression . (long_identifier_or_op (long_identifier (identifier)* . (identifier) @function.call)))
  ]
  .
  (_)? @variable)

; Method calls on objects
(application_expression
  .
  [
    (dot_expression base: (_) @variable.member field: (_) @function.call)
    (_ (dot_expression base: (_) @variable.member field: (_) @function.call))
    (_ (_ (dot_expression base: (_) @variable.member field: (_) @function.call)))
    (_ (_ (_ (dot_expression base: (_) @variable.member field: (_) @function.call))))
  ])

; ----------------------------------------------------------------------------
; Variables and parameters

(value_declaration_left . (_) @variable)
(primary_constr_args (_) @variable.parameter)
(argument_patterns) @variable.parameter
(typed_pattern
  (_pattern) @variable.parameter
  (_type) @type)
(class_as_reference (_) @variable.parameter.builtin)

; Underscore-prefixed identifiers are special
((argument_patterns (long_identifier (identifier) @character.special))
 (#match? @character.special "^\_.*"))

(wildcard_pattern) @character.special

; ----------------------------------------------------------------------------
; Fields and properties

(field_initializer field: (_) @property)
(record_fields
  (record_field . (identifier) @property))
(dot_expression
  base: (_) @variable
  field: (_) @variable.member)

; ----------------------------------------------------------------------------
; Computation expressions

(ce_expression . (_) @constant.macro)

; ----------------------------------------------------------------------------
; Patterns

(rules
  (rule
    pattern: (_) @constant
    block: (_)))

(identifier_pattern
  . (_) @constant
  . (_) @variable)

(optional_pattern "?" @character.special)

; ----------------------------------------------------------------------------
; Literals

[
  (xint)
  (int)
  (int16)
  (uint16)
  (int32)
  (uint32)
  (int64)
  (uint64)
  (nativeint)
  (unativeint)
] @number

[
  (ieee32)
  (ieee64)
  (float)
  (decimal)
] @number.float

(bool) @boolean

[
  (string)
  (triple_quoted_string)
  (verbatim_string)
  (char)
] @string

; ----------------------------------------------------------------------------
; Operators

[
  "|"
  "="
  ">"
  "<"
  "-"
  "~"
  "->"
  "<-"
  "&"
  "&&"
  "||"
  ":>"
  ":?>"
  ".."
  (infix_op)
  (prefix_op)
] @operator

; Pipe operators with function call highlighting
((infix_expression
  . (_)
  . (infix_op) @operator
  . (_) @function.call)
 (#eq? @operator "|>"))

((infix_expression
  . (_) @function.call
  . (infix_op) @operator
  . (_))
 (#eq? @operator "<|"))

; Type casts
(typecast_expression
  . (_) @variable
  . (_) @type)

; ----------------------------------------------------------------------------
; Punctuation

[
  "("
  ")"
  "{"
  "}"
  "["
  "]"
  "[|"
  "|]"
  "{|"
  "|}"
] @punctuation.bracket

[
  "[<"
  ">]"
] @punctuation.special

(format_string_eval
  [
    "{"
    "}"
  ] @punctuation.special)

[
  ","
  ";"
  ":"
  "."
] @punctuation.delimiter

(generic_type
  [
    "<"
    ">"
  ] @punctuation.bracket)

; ----------------------------------------------------------------------------
; Keywords

[
  "if"
  "then"
  "else"
  "elif"
  "when"
  "match"
  "match!"
] @keyword.conditional

[
  "and"
  "or"
  "not"
  "upcast"
  "downcast"
] @keyword.operator

[
  "return"
  "return!"
  "yield"
  "yield!"
] @keyword.return

[
  "for"
  "while"
  "downto"
  "to"
] @keyword.repeat

[
  "open"
  "#r"
  "#load"
] @keyword.import

[
  "abstract"
  "delegate"
  "static"
  "inline"
  "mutable"
  "override"
  "rec"
  "global"
  (access_modifier)
] @keyword.modifier

[
  "let"
  "let!"
  "use"
  "use!"
  "member"
] @keyword.function

[
  "try"
  "with"
  "finally"
] @keyword.exception

[
  "as"
  "assert"
  "begin"
  "end"
  "done"
  "default"
  "in"
  "do"
  "do!"
  "fun"
  "function"
  "get"
  "set"
  "lazy"
  "new"
  "null"
  "of"
  "struct"
  "val"
  "module"
  "namespace"
  "enum"
  "type"
  "exception"
  "inherit"
  "interface"
  "class"
] @keyword

; Exception functions
((identifier) @keyword.exception
 (#any-of? @keyword.exception "failwith" "failwithf" "raise" "reraise"))

(match_expression "with" @keyword.conditional)

(try_expression
  [
    "try"
    "with"
    "finally"
  ] @keyword.exception)

; ----------------------------------------------------------------------------
; Built-in types

((_type
  (long_identifier (identifier) @type.builtin))
 (#any-of? @type.builtin "bool" "byte" "sbyte" "int16" "uint16" "int" "uint"
   "int64" "uint64" "nativeint" "unativeint" "decimal" "float" "double"
   "float32" "single" "char" "string" "unit"))

; ----------------------------------------------------------------------------
; Built-in modules

((identifier) @module.builtin
 (#any-of? @module.builtin "Array" "Async" "Directory" "File" "List" "Option"
   "Path" "Map" "Set" "Lazy" "Seq" "Task" "String" "Result"))

; ----------------------------------------------------------------------------
; Preprocessor

(compiler_directive_decl) @keyword.directive

(preproc_line "#line" @keyword.directive)

(preproc_if
  [
    "#if" @keyword.directive
    "#endif" @keyword.directive
  ]
  condition: (_)? @keyword.directive)

(preproc_else "#else" @keyword.directive)

; ----------------------------------------------------------------------------
; Attributes

(attribute) @attribute

(attribute
  target: (identifier)? @keyword
  (_type) @attribute)

; Literal attribute marks constants
((value_declaration
   (attributes
     (attribute
       (_type
         (long_identifier
           (identifier) @attribute))))
   (function_or_value_defn
     (value_declaration_left
       .
       (_) @constant)))
 (#eq? @attribute "Literal"))

; ----------------------------------------------------------------------------
; Operators in identifiers

(long_identifier_or_op
  (op_identifier) @operator)
