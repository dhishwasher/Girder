import assert from 'node:assert';

export const alice = { /* m:alice.name */ name: () => 'literal' };
alice.name = () => 'mutated';

assert.strictEqual(alice.name(), 'mutated');
