import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './app.ts';

test('t', () => {
  // An import binding is immutable: the write throws and the call is unchanged.
  assert.throws(() => {
    // @ts-expect-error assignment to import
    target = () => 'replaced';
  }, TypeError);
  assert.strictEqual(/* claim */ target(), 'app.target');
});
