import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './\x61pp.ts';

test('t', () => {
  assert.strictEqual(/* claim */ target(), 'app.target');
});
