import { test, expect, vi } from 'vitest';
import { target } from './app.ts';

vi.mock(import('./app.ts'), () => ({ target: () => 'mocked' }));

test('t', () => {
  expect(/* claim */ target()).toBe('mocked');
});
