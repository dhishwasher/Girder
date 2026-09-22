import { test } from 'node:test';
import assert from 'node:assert';

class Circle {
  area(): number {
    return 1;
  }
}

class Square {
  area(): number {
    return 2;
  }
}

export function measure(shape: Circle | Square): number {
  return shape.area();
}

test('test_circle', () => {
  assert.strictEqual(measure(new Circle()), 1);
});
