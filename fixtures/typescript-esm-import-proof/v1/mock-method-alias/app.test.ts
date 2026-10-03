import { test, expect, vi } from 'vitest';
import { target } from './app.ts';

const { mock } = vi;
mock('./app.ts', () => ({ target: () => 'mocked' }));

test('t', () => {
  expect(/* claim */ target()).toBe('mocked');
});
