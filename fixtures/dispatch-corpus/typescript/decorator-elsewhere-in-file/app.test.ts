import { test } from 'node:test';
import assert from 'node:assert';

function logged(target: any, propertyKey: string, descriptor: PropertyDescriptor) {
  return descriptor;
}

export function target(): number {
  return 42;
}

test('test_direct', () => {
  assert.strictEqual(target(), 42);
});

class Unrelated {
  @logged
  method() {
    return 1;
  }
}
