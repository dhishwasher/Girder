import assert from 'node:assert';

export const api = {
  users: {
    /* m:api.users.list */ list: () => ['ada'],
    admins: {
      /* m:api.users.admins.list */ list() {
        return ['root'];
      },
    },
  },
};

assert.deepStrictEqual(api.users.list(), ['ada']);
assert.deepStrictEqual(api.users.admins.list(), ['root']);
