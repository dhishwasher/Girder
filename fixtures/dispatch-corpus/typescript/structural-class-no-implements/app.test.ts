import { test } from 'node:test';
import assert from 'node:assert';

interface Named {
  name(): string;
}

class Alice {
  name(): string {
    return 'alice';
  }
}

class Bob {
  name(): string {
    return 'bob';
  }
}

export function announce(n: Named): string {
  return n.name();
}

test('test_alice', () => {
  assert.strictEqual(announce(new Alice()), 'alice');
});

test('test_bob', () => {
  assert.strictEqual(announce(new Bob()), 'bob');
});
