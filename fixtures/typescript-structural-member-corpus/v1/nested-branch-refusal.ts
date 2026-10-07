import assert from 'node:assert';

function target() {
  return 'target';
}

export const api = {
  bad: /* b:nb.bad */ {
    'x-key': 1,
    /* m:nb.bad.list */ list: () => /* c:nb.bad.call */ target(),
    deeper: {
      /* m:nb.bad.deeper.list */ list: () => 'deeper',
    },
  },
  good: {
    /* m:nb.good.list */ list: () => 'good',
  },
};

assert.strictEqual(api.bad.list(), 'target');
assert.strictEqual(api.bad.deeper.list(), 'deeper');
assert.strictEqual(api.good.list(), 'good');
