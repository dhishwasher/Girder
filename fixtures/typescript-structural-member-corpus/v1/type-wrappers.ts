import assert from 'node:assert';

interface Named {
  name(): string;
}

export const sat = { /* m:sat.name */ name: () => 'sat' } satisfies Named;
export const cast = ({ /* m:cast.name */ name: () => 'cast' }) as Named;
export const paren = ({ /* m:paren.name */ name() { return 'paren'; } });

assert.strictEqual(sat.name(), 'sat');
assert.strictEqual(cast.name(), 'cast');
assert.strictEqual(paren.name(), 'paren');
