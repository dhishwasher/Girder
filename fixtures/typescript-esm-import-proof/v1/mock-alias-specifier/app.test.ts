import { test, expect, vi } from 'vitest';
import { target } from './app.ts';

test('t', () => {
  expect(/* claim */ target()).toBeTypeOf('string');
});
