import assert from 'node:assert';

/* m:rk.outer-helper */ function helper() {
  return 'outer helper';
}

export const cfg = /* b:rk.cfg */ {
  'x-key': 1,
  /* m:rk.run */ run: () => {
    /* m:rk.inner-helper */ function helper() {
      return 'inner helper';
    }
    const box = { /* m:rk.box.go */ go: () => /* c:rk.call */ helper() };
    return box.go();
  },
};

assert.strictEqual(cfg.run(), 'inner helper');
assert.strictEqual(helper(), 'outer helper');
