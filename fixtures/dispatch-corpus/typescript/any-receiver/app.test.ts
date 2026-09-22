import { test } from 'node:test';
import assert from 'node:assert';

export class Ops {
  add(a: number, b: number): number {
    return a + b;
  }
}

export function run(obj: any): number {
  return obj.add(2, 3);
}

test('test_any', () => {
  const ops = new Ops();
  assert.strictEqual(run(ops), 5);
});
