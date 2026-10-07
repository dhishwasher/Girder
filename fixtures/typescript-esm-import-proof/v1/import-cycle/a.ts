import { viaB } from './b.ts';

export function target(): string {
  return 'a.target';
}

export function useB(): string {
  return viaB();
}
