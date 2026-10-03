import { test } from 'node:test';
import assert from 'node:assert';

interface Named {
  name(): string;
}

function makeA(): Named {
  const alice = { name: () => 'a' };
  return alice;
}

function makeB(): Named {
  const alice = {
    name() {
      return 'b';
    },
  };
  return alice;
}

export function announce(n: Named): string {
  return n.name();
}

test('test_a', () => {
  assert.strictEqual(announce(makeA()), 'a');
});

test('test_b', () => {
  assert.strictEqual(announce(makeB()), 'b');
});
