(function_definition
  declarator: (function_declarator
    declarator: (identifier) @name)) @definition.function

(struct_specifier
  name: (type_identifier) @name
  body: (_)) @definition.class

(enum_specifier
  name: (type_identifier) @name
  body: (_)) @definition.class

(call_expression
  function: (identifier) @name) @reference.call
