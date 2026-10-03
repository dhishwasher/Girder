import { test } from 'node:test';
import assert from 'node:assert';
test('t', async () => {
  // Conflicting star exports make the named import a link-time SyntaxError.
  await assert.rejects(import('./consumer.ts'), SyntaxError);
});
