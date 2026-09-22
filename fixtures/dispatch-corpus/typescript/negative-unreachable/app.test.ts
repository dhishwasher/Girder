import { test } from 'node:test';
import assert from 'node:assert';

export function target(): number {
  return 42;
}

export function unrelated(): number {
  return 1;
}

test('test_unrelated', () => {
  assert.strictEqual(unrelated(), 1);
});
