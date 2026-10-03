import { test } from 'node:test';
import assert from 'node:assert';

export function target(): string {
  return 'target';
}

export function host(): string {
  // `let` owner and a non-plain key: both refuse this literal's members.
  let cfg = {
    'x-key': 1,
    run: () => {
      function relay(): string {
        return target();
      }
      return relay();
    },
  };
  return cfg.run();
}

export function unrelated(): string {
  return 'unrelated';
}

test('test_host', () => {
  assert.strictEqual(host(), 'target');
});

test('test_unrelated', () => {
  assert.strictEqual(unrelated(), 'unrelated');
});
