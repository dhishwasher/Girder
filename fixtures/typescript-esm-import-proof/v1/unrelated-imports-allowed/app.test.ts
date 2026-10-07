import { test } from 'node:test';
import assert from 'node:assert';
import { join } from 'node:path';
import type { Shape } from './types.ts';
import { other } from './other.ts';
import './side.ts';
import { target } from './app.ts';

const shape: Shape = { name: join('a', 'b') };

test('t', () => {
  assert.strictEqual(shape.name, 'a/b');
  assert.strictEqual(other(), 'other');
  assert.strictEqual(/* claim */ target(), 'app.target');
});
