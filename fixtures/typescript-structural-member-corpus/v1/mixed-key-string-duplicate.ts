import assert from 'node:assert';

// A plain key and a string key spell the same property; the string one wins.
export const dup = {
  /* m:sd.plain */ name: () => 'plain',
  /* m:sd.string */ 'name': () => 'string',
  /* m:sd.sibling */ other() {
    return 'other';
  },
};

// A separate literal with only plain keys keeps its identities.
export const safe = { /* m:sd.safe */ name: () => 'safe' };

assert.strictEqual(dup.name(), 'string');
assert.strictEqual(dup.other(), 'other');
assert.strictEqual(safe.name(), 'safe');
