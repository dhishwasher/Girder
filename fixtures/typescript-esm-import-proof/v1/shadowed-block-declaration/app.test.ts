import { test } from 'node:test';
import assert from 'node:assert';
import { target } from './app.ts';

test('t', () => {
  {
    function target(): string {
      return 'inner';
    }
    assert.strictEqual(/* claim */ target(), 'inner');
  }
});
