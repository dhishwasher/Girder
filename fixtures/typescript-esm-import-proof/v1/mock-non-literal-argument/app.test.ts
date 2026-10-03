import { test, expect, vi } from 'vitest';
import { target } from './app.ts';

const path = './other.ts';
vi.mock(path, () => ({ other: () => 'mocked' }));

test('t', () => {
  expect(/* claim */ target()).toBe('app.target');
});
