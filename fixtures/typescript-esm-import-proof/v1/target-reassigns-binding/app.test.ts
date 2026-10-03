import { test } from 'node:test';
import assert from 'node:assert';
import { target, swap } from './app.ts';

test('t', () => {
  swap();
  // Imports are live bindings: the importer sees the reassignment.
  assert.strictEqual(/* claim */ target(), 'swapped');
});
