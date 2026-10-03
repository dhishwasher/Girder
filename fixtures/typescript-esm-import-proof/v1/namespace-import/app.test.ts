import { test } from 'node:test';
import assert from 'node:assert';
import * as app from './app.ts';

test('t', () => {
  assert.strictEqual(/* claim */ app.target(), 'app.target');
});
