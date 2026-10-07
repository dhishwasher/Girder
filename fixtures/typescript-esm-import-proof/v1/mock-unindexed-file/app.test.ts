import { test, mock } from 'node:test';
import assert from 'node:assert';
import { target } from './app.ts';

mock.module('./target/helper.ts', { namedExports: { helper: () => 'mocked' } });

test('t', () => {
  assert.strictEqual(/* claim */ target(), 'app.target');
});
