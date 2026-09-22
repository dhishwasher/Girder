import { test } from 'node:test';
import assert from 'node:assert';

export function target(): number {
  return 42;
}

export function run(callback: () => number): number {
  return callback();
}

test('test_via_callback', () => {
  const cb: () => number = target;
  assert.strictEqual(run(cb), 42);
});
