import { test } from 'node:test';
import assert from 'node:assert';
import { join } from 'node:path';

test('t', () => {
  assert.strictEqual(/* claim */ join('a', 'b'), 'a/b');
});
