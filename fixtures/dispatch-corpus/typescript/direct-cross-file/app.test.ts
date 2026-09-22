import { test } from 'node:test';
import assert from 'node:assert';

import { target } from './app.ts';

test('test_cross_file', () => {
  assert.strictEqual(target(), 42);
});
