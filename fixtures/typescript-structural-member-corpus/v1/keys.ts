import assert from 'node:assert';

const key = 'computed';
export const odd = {
  /* m:string-key */ 'name': () => 'string',
  /* m:numeric-key */ 0: () => 'numeric',
  /* m:computed-key */ [key]: () => 'computed',
  /* m:string-method */ 'run'() {
    return 'string method';
  },
};

assert.strictEqual(odd.name(), 'string');
assert.strictEqual(odd[0](), 'numeric');
assert.strictEqual(odd.computed(), 'computed');
assert.strictEqual(odd.run(), 'string method');
