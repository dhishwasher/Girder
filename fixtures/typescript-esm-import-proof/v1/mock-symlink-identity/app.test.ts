import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './real/app.ts';

test('t', () => {
  assert.strictEqual(/* claim */ target(), 'mocked');
});
