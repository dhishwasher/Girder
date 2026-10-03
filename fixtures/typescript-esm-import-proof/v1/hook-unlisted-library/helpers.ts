import esmock from 'esmock';

export const load = (path: string, mocks: object) => esmock(path, mocks);
