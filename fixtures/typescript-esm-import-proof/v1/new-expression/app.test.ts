import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './app.ts';

test('t', () => {
  // @ts-expect-error constructing a function declaration
  assert.strictEqual((/* claim */ new target()).tag, 'app.target');
});
