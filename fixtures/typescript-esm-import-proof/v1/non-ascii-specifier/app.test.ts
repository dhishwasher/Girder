import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './äpp.ts';

test('t', () => {
  assert.strictEqual(/* claim */ target(), 'umlaut.target');
});
