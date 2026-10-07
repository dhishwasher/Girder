import assert from 'node:assert';

export const alice = { /* m:alice.name */ name: () => 'alice' };
export const asyncy = { /* m:asyncy.load */ load: async () => 'loaded' };

assert.strictEqual(alice.name(), 'alice');
assert.strictEqual(alice.name.name, 'name');
