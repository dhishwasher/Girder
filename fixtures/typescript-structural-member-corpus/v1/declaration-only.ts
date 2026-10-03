export interface Named {
  /* d:interface.method_signature */ name(): string;
  /* d:interface.function_property_signature */ label: () => string;
  /* d:interface.call_signature */ (): void;
  /* d:interface.construct_signature */ new (): Named;
  /* d:interface.index_signature */ [key: string]: unknown;
}

export type Shape = {
  /* d:type_literal.method_signature */ area(): number;
};

export abstract class Base {
  /* d:abstract_method_signature */ abstract run(): void;
  /* d:class.overload_signature */ step(): void;
  step(n?: number): void {
    void n;
  }
}

/* d:function_signature */ export function over(): void;
export function over(n?: number): void {
  void n;
}

/* d:ambient_function */ declare function ambient(): void;
