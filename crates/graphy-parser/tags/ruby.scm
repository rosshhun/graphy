(method
  name: (identifier) @name) @definition.method

(singleton_method
  name: (identifier) @name) @definition.method

(class
  name: (constant) @name) @definition.class

(module
  name: (constant) @name) @definition.module

(call
  method: (identifier) @name) @reference.call
