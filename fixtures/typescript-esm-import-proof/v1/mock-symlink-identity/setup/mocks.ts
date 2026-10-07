import { mock } from 'node:test';

mock.module('../lib/app.ts', { namedExports: { target: () => 'mocked' } });
