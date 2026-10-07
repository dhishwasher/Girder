import assert from 'node:assert';

export const dup = {
  /* m:dup.first */ name: () => 'first',
  /* m:dup.second */ name() {
    return 'second';
  },
};

assert.strictEqual(dup.name(), 'second');
