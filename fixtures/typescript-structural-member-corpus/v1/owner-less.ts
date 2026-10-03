import assert from 'node:assert';

function announce(n: { name(): string }): string {
  return n.name();
}

function factory() {
  return {
    /* m:returned.name */ name() {
      return 'returned';
    },
  };
}

assert.strictEqual(announce({ /* m:argument.name */ name: () => 'argument' }), 'argument');
assert.strictEqual(factory().name(), 'returned');
export const list = [{ /* m:element.name */ name: () => 'element' }];
export function withDefault(n = { /* m:default.name */ name() { return 'default'; } }) {
  return n.name();
}
assert.strictEqual(list[0].name(), 'element');
assert.strictEqual(withDefault(), 'default');
