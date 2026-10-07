import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './app.ts';

test('t', () => {
  eval('void 0');
  assert.strictEqual(/* claim */ target(), 'app.target');
});
