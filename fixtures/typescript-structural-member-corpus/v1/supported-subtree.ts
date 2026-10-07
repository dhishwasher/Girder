import assert from 'node:assert';

/* m:ss.outer-helper */ function helper() {
  return 'outer helper';
}

// Positive control: a supported member's descendants are scoped under it,
// so the inner helper cannot collide with the outer one.
export const ok = {
  /* m:ss.run */ run: () => {
    /* m:ss.inner-helper */ function helper() {
      return 'inner helper';
    }
    const box = { /* m:ss.box.go */ go: () => helper() };
    return box.go();
  },
};

assert.strictEqual(ok.run(), 'inner helper');
assert.strictEqual(helper(), 'outer helper');
