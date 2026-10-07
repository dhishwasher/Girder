let calls = 0;
export function target(): string {
  calls += 1;
  return `app.target#${calls}`;
}
