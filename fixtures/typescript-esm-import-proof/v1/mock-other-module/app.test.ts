import { test, mock } from 'node:test';
import assert from 'node:assert';
import { target } from './app.ts';

mock.module('./other.ts', { namedExports: { other: () => 'mocked other' } });

test('t', async () => {
  assert.strictEqual(/* claim */ target(), 'app.target');
  assert.strictEqual((await import('./other.ts')).other(), 'mocked other');
});
