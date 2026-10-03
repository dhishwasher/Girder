import assert from 'node:assert';

/* m:ru.outer-helper */ function helper() {
  return 'outer helper';
}

/* m:ru.host */ export function host() {
  let cfg = /* b:ru.cfg */ {
    /* m:ru.cfg.run */ run: () => {
      /* m:ru.inner-helper */ function helper() {
        return 'inner helper';
      }
      const box = { /* m:ru.box.go */ go: () => /* c:ru.call */ helper() };
      return box.go();
    },
  };
  const ok = { /* m:ru.ok.run */ run: () => 'ok' };
  return [cfg.run(), ok.run()];
}

assert.deepStrictEqual(host(), ['inner helper', 'ok']);
assert.strictEqual(helper(), 'outer helper');
