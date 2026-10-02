function target() { return 'original'; }
export function probe() { return /* claim */ target(); }
