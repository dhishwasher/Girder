import { test } from 'node:test';
import assert from 'node:assert';

interface Named {
  name(): string;
}

const alice: Named = { name: () => 'alice' };
export const dave = { title: () => 'dave' };

export function announce(n: Named): string {
  return n.name();
}

test('test_alice', () => {
  assert.strictEqual(announce(alice), 'alice');
});
