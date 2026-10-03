import assert from 'node:assert';

export const { pulled } = { /* m:destructured.pulled */ pulled: () => 'pulled' };
export class Holder {
  cfg = { /* m:class-field.run */ run: () => 'field' };
}
export const assigned: { run?: () => string } = {};
assigned.run = () => 'assigned';

assert.strictEqual(pulled(), 'pulled');
assert.strictEqual(new Holder().cfg.run(), 'field');
assert.strictEqual(assigned.run!(), 'assigned');
