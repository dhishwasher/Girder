import assert from 'node:assert';

export function make() {
  const alice = { /* m:make.alice.name */ name: () => 'alice' };
  return alice;
}

export const build = () => {
  const alice = {
    /* m:build.alice.name */ name() {
      return 'built';
    },
  };
  return alice;
};

assert.strictEqual(make().name(), 'alice');
assert.strictEqual(build().name(), 'built');
