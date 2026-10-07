import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './app.ts?instance=2';
import { target as plain } from './app.ts';

test('t', () => {
  assert.strictEqual(plain(), 'app.target#1');
  // A query creates a distinct module instance with its own state.
  assert.strictEqual(/* claim */ target(), 'app.target#1');
});
