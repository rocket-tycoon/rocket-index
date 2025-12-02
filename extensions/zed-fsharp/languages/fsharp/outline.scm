; Outline queries for F# in Zed
; These queries define what appears in the symbol outline/breadcrumbs

; Module definitions
(module_defn
  (long_identifier) @name) @item

; Namespace definitions
(namespace
  (long_identifier) @name) @item

; Function definitions
(function_or_value_defn
  (function_declaration_left
    (identifier) @name)) @item

; Value definitions (let bindings without parameters)
(function_or_value_defn
  (value_declaration_left
    (identifier_pattern
      (long_identifier
        (identifier) @name)))) @item

; Type definitions - Record
(type_definition
  (record_type_defn
    (identifier) @name)) @item

; Type definitions - Union/Discriminated Union
(type_definition
  (union_type_defn
    (identifier) @name)) @item

; Type definitions - Class
(type_definition
  (class_type_defn
    (identifier) @name)) @item

; Type definitions - Interface
(type_definition
  (interface_type_defn
    (identifier) @name)) @item

; Type definitions - Type abbreviation
(type_definition
  (type_abbrev_defn
    (identifier) @name)) @item

; Type definitions - Type extension
(type_definition
  (type_extension
    (long_identifier) @name)) @item

; Type definitions - Enum
(type_definition
  (enum_type_defn
    (identifier) @name)) @item

; Type definitions - Struct
(type_definition
  (struct_type_defn
    (identifier) @name)) @item

; Member definitions within types
(member_defn
  (identifier) @name) @item

; Property definitions
(property_or_ident
  (identifier) @name) @item

; Union type cases
(union_type_case
  (identifier) @name) @item

; Exception definitions
(exception_defn
  (identifier) @name) @item
