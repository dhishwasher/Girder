export function target(): string {
  return 'app.target';
}
export function swap(): void {
  target = () => 'swapped';
}
