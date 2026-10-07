import { test, mock } from 'node:test';
import assert from 'node:assert';
import { target } from './app.ts';

mock.module('./lib.ts', { namedExports: { lib: () => 'mocked' } });

test('t', () => {
  assert.strictEqual(/* claim */ target(), 'app.target');
});
