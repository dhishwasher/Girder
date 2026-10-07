import { test } from 'node:test';
import assert from 'node:assert';

interface Named {
  name(): string;
}

const base: Named = { name: () => 'base' };
const alice: Named = { name: () => 'alice', ...base };

export function announce(n: Named): string {
  return n.name();
}

test('test_alice', () => {
  assert.strictEqual(announce(alice), 'base');
});
