import { test } from 'node:test';
import assert from 'node:assert';

class Shape {
  area(): number {
    return 0;
  }
}

class Square extends Shape {
  area(): number {
    return 4;
  }
}

export function render(shape: Shape): number {
  return shape.area();
}

test('test_square', () => {
  assert.strictEqual(render(new Square()), 4);
});

test('test_base_shape', () => {
  assert.strictEqual(render(new Shape()), 0);
});
