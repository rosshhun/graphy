(class_declaration
  name: (identifier) @name) @definition.class

(interface_declaration
  name: (identifier) @name) @definition.interface

(method_declaration
  name: (identifier) @name) @definition.method

(constructor_declaration
  name: (identifier) @name) @definition.method

(invocation_expression
  function: (identifier) @name) @reference.call

(invocation_expression
  function: (member_access_expression
    name: (identifier) @name)) @reference.call

(attribute
  name: (identifier) @name) @definition.decorator
