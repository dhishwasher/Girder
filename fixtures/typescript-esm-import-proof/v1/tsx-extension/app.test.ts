import { test } from 'node:test';
import assert from 'node:assert';
test('t', async () => {
  await assert.rejects(import('./consumer.ts'), { code: 'ERR_UNKNOWN_FILE_EXTENSION' });
});
