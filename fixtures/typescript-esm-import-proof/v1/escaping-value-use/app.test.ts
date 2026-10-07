import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './app.ts';

const registry = [target];

test('t', () => {
  assert.strictEqual(registry[0](), 'app.target');
  assert.strictEqual(/* claim */ target(), 'app.target');
});
