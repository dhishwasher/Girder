import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './app.ts';

test('t', async () => {
  assert.strictEqual(await /* claim */ target(), 'app.target');
});
