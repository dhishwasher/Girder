export function target(): string {
  return 'app.target';
}
function other(): string {
  return 'other';
}
export { other as target };
