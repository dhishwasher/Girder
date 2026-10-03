import { test } from 'node:test';
import assert from 'node:assert';
import { target } from '../lib/app.ts';

test('t', () => {
  assert.strictEqual(/* claim */ target(), 'app.target');
});
