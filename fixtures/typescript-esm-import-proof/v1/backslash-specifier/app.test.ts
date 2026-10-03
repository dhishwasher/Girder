import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './sub\\app.ts';

test('t', () => {
  assert.strictEqual(/* claim */ target(), 'sub.app');
});
