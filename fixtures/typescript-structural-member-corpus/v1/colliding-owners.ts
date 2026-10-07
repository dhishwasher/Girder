import assert from 'node:assert';

{
  const alice = { /* m:block-1.name */ name: () => 'one' };
  assert.strictEqual(alice.name(), 'one');
}
{
  const alice = { /* m:block-2.name */ name: () => 'two' };
  assert.strictEqual(alice.name(), 'two');
}
export {};
