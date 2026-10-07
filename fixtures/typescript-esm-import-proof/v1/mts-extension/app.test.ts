import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './lib.mts';

test('t', () => {
  assert.strictEqual(/* claim */ target(), 'lib.target');
});
