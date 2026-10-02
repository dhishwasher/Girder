export function probe() { function target() { return 'original'; } const invoke = () => /* claim */ target(); return invoke(); }
