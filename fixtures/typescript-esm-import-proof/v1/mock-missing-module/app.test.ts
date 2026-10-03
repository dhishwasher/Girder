import { test, mock } from 'node:test';
import assert from 'node:assert';
import { target } from './app.ts';

test('t', () => {
  assert.throws(() => mock.module('./missing.ts', { namedExports: {} }), { code: 'ERR_MODULE_NOT_FOUND' });
  assert.strictEqual(/* claim */ target(), 'app.target');
});
