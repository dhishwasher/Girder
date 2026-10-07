import { mock } from 'node:test';

mock.module('../app.ts', { namedExports: { target: () => 'mocked' } });
