import assert from 'node:assert';

export const alice = {
  /* m:alice.name */ name: function () {
    return 'alice';
  },
  /* m:alice.label */ label: function inner() {
    return 'label';
  },
};

assert.strictEqual(alice.name(), 'alice');
assert.strictEqual(alice.name.name, 'name');
assert.strictEqual(alice.label(), 'label');
assert.strictEqual(alice.label.name, 'inner');
