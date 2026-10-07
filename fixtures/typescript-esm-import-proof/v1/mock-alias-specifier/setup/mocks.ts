import { vi } from 'vitest';

vi.mock('@/app', () => ({ target: () => 'mocked' }));
