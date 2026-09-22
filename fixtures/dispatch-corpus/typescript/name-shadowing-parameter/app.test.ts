import { test } from 'node:test';
import assert from 'node:assert';

export function target(): number {
  return 42;
}

export function run(target: () => number): number {
  return target();
}

test('test_shadowed', () => {
  assert.strictEqual(run(target), 42);
});
