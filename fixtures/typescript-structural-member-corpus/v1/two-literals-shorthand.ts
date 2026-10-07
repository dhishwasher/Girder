import assert from 'node:assert';

export const alice = {
  /* m:alice.name */ name() {
    return 'alice';
  },
};

export const bob = {
  /* m:bob.name */ name() {
    return 'bob';
  },
};

assert.strictEqual(alice.name(), 'alice');
assert.strictEqual(bob.name(), 'bob');
