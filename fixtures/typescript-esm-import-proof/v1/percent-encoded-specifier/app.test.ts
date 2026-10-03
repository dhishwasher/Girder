import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './%61pp.ts';

test('t', () => {
  assert.strictEqual(/* claim */ target(), 'app.target');
});
