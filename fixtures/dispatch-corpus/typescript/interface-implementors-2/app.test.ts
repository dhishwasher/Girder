import { test } from 'node:test';
import assert from 'node:assert';

export interface Greeter {
  greet(): string;
}

export class English implements Greeter {
  greet(): string {
    return 'hello';
  }
}

export class French implements Greeter {
  greet(): string {
    return 'bonjour';
  }
}

export function dispatch(g: Greeter): string {
  return g.greet();
}

test('test_via_english', () => {
  assert.strictEqual(dispatch(new English()), 'hello');
});

test('test_via_french', () => {
  assert.strictEqual(dispatch(new French()), 'bonjour');
});
