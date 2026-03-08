(function_definition
  name: (name) @name) @definition.function

(method_declaration
  name: (name) @name) @definition.method

(class_declaration
  name: (name) @name) @definition.class

(interface_declaration
  name: (name) @name) @definition.interface

(function_call_expression
  function: (name) @name) @reference.call

(member_call_expression
  name: (name) @name) @reference.call
