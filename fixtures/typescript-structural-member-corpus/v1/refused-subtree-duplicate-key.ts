import assert from 'node:assert';

/* m:rd.outer-helper */ function helper() {
  return 'outer helper';
}

export const dup = {
  /* m:rd.first */ run: () => 'first',
  /* m:rd.second */ run: () => {
    /* m:rd.inner-helper */ function helper() {
      return 'inner helper';
    }
    const box = { /* m:rd.box.go */ go: () => /* c:rd.call */ helper() };
    return box.go();
  },
  /* m:rd.sibling */ other: () => 'other',
};

export const safe = { /* m:rd.safe.run */ run: () => helper() };

assert.strictEqual(dup.run(), 'inner helper');
assert.strictEqual(safe.run(), 'outer helper');
