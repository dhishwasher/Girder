export const zed = {
  /* m:zed.name */ name: () => 'inserted literal above',
};

export const alice = {
  /* m:alice.extra */ extra: () => 'inserted sibling',
  /* m:alice.greet */ greet: () => 'hello, edited body and form',
  /* m:alice.name */ name() {
    return 'alice, edited body and form';
  },
};

export const bob = {
  /* m:bob.name */ name: () => 'bob, form changed',
};
