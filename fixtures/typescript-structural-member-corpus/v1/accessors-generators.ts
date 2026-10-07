import assert from 'node:assert';

export const special = {
  /* m:getter */ get name() {
    return () => 'from getter';
  },
  /* m:setter */ set label(value: string) {
    void value;
  },
  /* m:generator-method */ *gen() {
    yield 1;
  },
  /* m:generator-expression */ gen2: function* () {
    yield 2;
  },
  /* m:async-generator-method */ async *agen() {
    yield 3;
  },
};

assert.strictEqual(special.name(), 'from getter');
assert.deepStrictEqual([...special.gen()], [1]);
assert.deepStrictEqual([...special.gen2()], [2]);
