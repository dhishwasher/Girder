import { target } from './a.ts';

export function viaB(): string {
  return /* claim */ target();
}
