export const alice = {
  /* m:alice.name */ name: () => 'alice',
  /* m:alice.greet */ greet() {
    return 'hi';
  },
};

export const bob = {
  /* m:bob.name */ name: function () {
    return 'bob';
  },
};
