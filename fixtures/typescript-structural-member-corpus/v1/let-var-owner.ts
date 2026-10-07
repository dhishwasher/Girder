import assert from 'node:assert';

export let mutable = { /* m:let.name */ name: () => 'let' };
export var legacy = {
  /* m:var.name */ name() {
    return 'var';
  },
};

assert.strictEqual(mutable.name(), 'let');
assert.strictEqual(legacy.name(), 'var');
mutable = { name: () => 'replaced' };
assert.strictEqual(mutable.name(), 'replaced');
