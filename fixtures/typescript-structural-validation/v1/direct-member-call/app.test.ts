import { test } from 'node:test';
import assert from 'node:assert';

const alice = { name: () => 'alice' };

test('test_direct', () => {
  assert.strictEqual(alice.name(), 'alice');
});
