import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './ap	p.ts';

test('t', () => {
  assert.strictEqual(/* claim */ target(), 'app.target');
});
