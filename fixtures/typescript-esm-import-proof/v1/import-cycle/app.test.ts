import { test } from 'node:test';
import assert from 'node:assert';
import { useB } from './a.ts';

test('t', () => {
  // Function declarations are initialized before cyclic evaluation runs.
  assert.strictEqual(useB(), 'a.target');
});
