import assert from 'node:assert';

const key = 'name';

// A computed key aliases the plain key at runtime; the computed one wins.
export const aliased = {
  /* m:ca.plain */ name: () => 'plain',
  /* m:ca.computed */ [key]: () => 'computed',
  /* m:ca.sibling */ other: () => 'other',
};

// An escaped identifier spelling aliases a plain key; the later one wins.
export const escaped = {
  /* m:ca.escaped-plain */ name: () => 'plain',
  /* m:ca.escaped */ name: () => 'escaped',
  /* m:ca.escaped-sibling */ run() {
    return 'run';
  },
};

// An escaped spelling alone (no plain twin) still refuses its literal.
export const escapedOnly = {
  /* m:ca.escaped-only */ list: () => 'escaped only',
  /* m:ca.escaped-only-sibling */ size: () => 1,
};

export const safe = { /* m:ca.safe */ name: () => 'safe' };

assert.strictEqual(aliased.name(), 'computed');
assert.strictEqual(aliased.other(), 'other');
assert.strictEqual(escaped.name(), 'escaped');
assert.strictEqual(escaped.run(), 'run');
assert.strictEqual(escapedOnly.list(), 'escaped only');
assert.strictEqual(safe.name(), 'safe');
