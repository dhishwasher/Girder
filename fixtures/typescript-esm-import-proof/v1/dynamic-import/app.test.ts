import { test } from 'node:test';
import assert from 'node:assert';
test('t', async () => {
  const { target } = await import('./app.ts');
  assert.strictEqual(/* claim */ target(), 'app.target');
});
