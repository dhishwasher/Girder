function target() { return 'original'; }
export function probe() { const escaped = { target }; return /* claim */ target(); }
