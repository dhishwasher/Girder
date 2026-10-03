import { test } from 'node:test';
import assert from 'node:assert';
import { type target } from './app.ts';

test('t', () => {
  // Type-only imports are erased: no runtime binding exists.
  assert.throws(() => /* claim */ target(), ReferenceError);
});
