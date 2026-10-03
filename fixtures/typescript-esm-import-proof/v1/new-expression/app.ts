export function target(this: { tag?: string }): void {
  this.tag = 'app.target';
}
