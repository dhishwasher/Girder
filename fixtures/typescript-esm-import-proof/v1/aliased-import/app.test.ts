import { test } from 'node:test';
import assert from 'node:assert';
import { target as t } from './app.ts';

test('t', () => {
  assert.strictEqual(/* claim */ t(), 'app.target');
});
