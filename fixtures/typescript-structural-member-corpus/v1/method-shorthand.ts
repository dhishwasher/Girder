import assert from 'node:assert';

export const alice = {
  /* m:alice.name */ name() {
    return 'alice';
  },
  /* m:alice.load */ async load() {
    return 'loaded';
  },
};

assert.strictEqual(alice.name(), 'alice');
