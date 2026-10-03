import assert from 'node:assert';

/* m:rc.outer-helper */ function helper() {
  return 'outer helper';
}

{
  const alice = {
    /* m:rc.block-1.name */ name: () => {
      /* m:rc.block-1.inner-helper */ function helper() {
        return 'inner 1';
      }
      const /* m:rc.block-1.local */ local = () => /* c:rc.block-1.call */ helper();
      const box = { /* m:rc.block-1.box.go */ go: () => local() };
      return box.go();
    },
  };
  assert.strictEqual(alice.name(), 'inner 1');
}
{
  const alice = {
    /* m:rc.block-2.name */ name: () => {
      /* m:rc.block-2.inner-helper */ function helper() {
        return 'inner 2';
      }
      const box = { /* m:rc.block-2.box.go */ go: () => /* c:rc.block-2.call */ helper() };
      return box.go();
    },
  };
  assert.strictEqual(alice.name(), 'inner 2');
}
assert.strictEqual(helper(), 'outer helper');
export {};
