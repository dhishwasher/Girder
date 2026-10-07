import assert from 'node:assert';

// A nested function declaration and an object-literal member with the same
// owner and key spelling live in different block scopes. Their identities
// must differ by construction, not by luck of the redeclaration rules.
{
  function alice() {
    /* m:alice.nested-function */ function name() {
      return 'nested function';
    }
    return name();
  }
  assert.strictEqual(alice(), 'nested function');
}
{
  const alice = {
    /* m:alice.member */ name() {
      return 'member';
    },
  };
  assert.strictEqual(alice.name(), 'member');
}
export {};
