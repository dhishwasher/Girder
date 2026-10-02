function target() { return 'original'; }
export function probe() { eval("target = () => 'alternate'"); return /* claim */ target(); }
