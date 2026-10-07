import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './app.ts';

test('t', () => {
  assert.strictEqual(/* claim */ t\u0061rget(), 'app.target');
});
