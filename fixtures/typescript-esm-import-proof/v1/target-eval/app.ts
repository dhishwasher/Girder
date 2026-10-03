export function target(): string {
  return 'app.target';
}
eval("target = () => 'evaled'");
