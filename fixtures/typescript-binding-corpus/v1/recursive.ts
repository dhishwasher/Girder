export function probe() { function target(n = 2) { return n ? /* claim */ target(n - 1) : 'original'; } return target(); }
