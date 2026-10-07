import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './lib.cts';

test('t', () => {
  assert.strictEqual(/* claim */ target(), 'cts.target');
});
