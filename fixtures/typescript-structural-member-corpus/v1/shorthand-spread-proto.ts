import assert from 'node:assert';

function name() {
  return 'declared function';
}
const base = { /* m:base.name */ name: () => 'base' };
const proto = { inherited: () => 'proto' };

export const short = { /* m:shorthand-property */ name };
export const spreadAfter = { /* m:spread-after.name */ name: () => 'own', ...base };
export const spreadBefore = { ...base, /* m:spread-before.name */ name: () => 'own' };
export const withProto = { __proto__: proto, /* m:proto.name */ name: () => 'own' };
export const stringProto = { '__proto__': proto, /* m:string-proto.name */ name: () => 'own' };

assert.strictEqual(short.name(), 'declared function');
assert.strictEqual(spreadAfter.name(), 'base');
assert.strictEqual(spreadBefore.name(), 'own');
assert.strictEqual((withProto as any).inherited(), 'proto');
assert.strictEqual((stringProto as any).inherited(), 'proto');
