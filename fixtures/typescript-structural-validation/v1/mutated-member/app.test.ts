import { test } from 'node:test';
import assert from 'node:assert';

interface Named {
  name(): string;
}

const alice: Named = { name: () => 'literal' };
alice.name = () => 'mutated';

export function announce(n: Named): string {
  return n.name();
}

test('test_mutated', () => {
  assert.strictEqual(announce(alice), 'mutated');
});
