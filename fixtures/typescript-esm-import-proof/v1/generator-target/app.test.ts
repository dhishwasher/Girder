import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './app.ts';

test('t', () => {
  assert.deepStrictEqual([.../* claim */ target()], ['app.target']);
});
