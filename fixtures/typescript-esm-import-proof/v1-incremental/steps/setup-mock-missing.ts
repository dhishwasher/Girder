import { mock } from 'node:test';

mock.module('../missing.ts', { namedExports: {} });
