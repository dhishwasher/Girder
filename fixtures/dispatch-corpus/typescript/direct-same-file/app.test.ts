import { test } from 'node:test';
import assert from 'node:assert';

export function target(): number {
  return 42;
}

test('test_direct', () => {
  assert.strictEqual(target(), 42);
});
