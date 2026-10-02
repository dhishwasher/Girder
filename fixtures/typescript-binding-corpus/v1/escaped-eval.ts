function target() { return 'original'; }
export function probe() { \u0065val("target = () => 'alternate'"); return /* claim */ target(); }
