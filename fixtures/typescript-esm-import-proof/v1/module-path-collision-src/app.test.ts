import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './src/app.ts';

test('t', () => {
  assert.strictEqual(/* claim */ target(), 'src.target');
});
