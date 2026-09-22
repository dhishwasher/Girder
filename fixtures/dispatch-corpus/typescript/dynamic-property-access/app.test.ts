import { test } from 'node:test';
import assert from 'node:assert';

export class Ops {
  add(a: number, b: number): number {
    return a + b;
  }
}

export function dispatch(obj: Ops, key: string, a: number, b: number): number {
  return (obj as any)[key](a, b);
}

test('test_dynamic_key', () => {
  const ops = new Ops();
  assert.strictEqual(dispatch(ops, 'add', 2, 3), 5);
});
