export function target(): string;
export function target(x: number): string;
export function target(x?: number): string {
  return x === undefined ? 'app.target' : 'app.number';
}
