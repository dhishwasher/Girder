import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './app.ts';

function run(target: () => string): string {
  return /* claim */ target();
}

test('t', () => {
  assert.strictEqual(run(() => 'param'), 'param');
  assert.strictEqual(target(), 'app.target');
});
