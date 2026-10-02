export function probe() { function target() { return 'original'; } return /* claim */ target(); }
