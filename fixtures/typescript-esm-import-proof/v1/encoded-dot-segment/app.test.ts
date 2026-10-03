import { test } from 'node:test';
import assert from 'node:assert';
import { value } from './sub/consumer.ts';

test('t', () => {
  assert.strictEqual(value(), 'app.target');
});
