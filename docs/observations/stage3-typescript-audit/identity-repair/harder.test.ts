import { test } from 'node:test';
function first() {}
function second() {}
function third() {}
function fourth() {}
function fifth() {}
describe('same', () => {
  it('duplicate', () => { function local() { first(); } local(); });
  test('duplicate', () => { function local() { second(); } local(); });
});
describe('same', () => {
  it('duplicate', () => { third(); });
});
test('a::b', () => { fourth(); });
test('a:b', () => { fifth(); });
