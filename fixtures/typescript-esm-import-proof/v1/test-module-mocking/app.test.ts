import { test, expect, vi } from 'vitest';
import { target } from './app.ts';

// vitest hoists vi.mock above imports, replacing the module the import binds to.
vi.mock('./app.ts', () => ({ target: () => 'mocked' }));

test('t', () => {
  expect(/* claim */ target()).toBe('mocked');
});
