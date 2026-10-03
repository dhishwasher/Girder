interface Named {
  name(): string;
}

export const angled = <Named>{ /* m:angled.name */ name: () => 'angled' };
