function target(n: number): string;
function target(n: number) { return 'original'; }
export function probe() { return /* claim */ target(1); }
