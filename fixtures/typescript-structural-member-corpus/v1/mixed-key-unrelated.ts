import assert from 'node:assert';

// Each literal has one unsupported key on a property unrelated to its
// callable members. All callable members of that literal are refused.
export const withString = {
  /* m:ur.string.name */ name: () => 'plain',
  'data-id': 7,
};
export const withNumeric = {
  /* m:ur.numeric.run */ run() {
    return 'run';
  },
  0: 'zero',
};
export const withSymbol = {
  /* m:ur.symbol.go */ go: () => 'go',
  /* m:ur.symbol.iterator */ *[Symbol.iterator]() {
    yield 1;
  },
};

export const safe = {
  /* m:ur.safe.name */ name: () => 'safe',
  /* m:ur.safe.run */ run() {
    return 'safe run';
  },
};

assert.strictEqual(withString.name(), 'plain');
assert.strictEqual(withString['data-id'], 7);
assert.strictEqual(withNumeric.run(), 'run');
assert.strictEqual(withSymbol.go(), 'go');
assert.deepStrictEqual([...withSymbol], [1]);
assert.strictEqual(safe.name(), 'safe');
assert.strictEqual(safe.run(), 'safe run');
