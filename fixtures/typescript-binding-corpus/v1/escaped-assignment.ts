function target() { return 'original'; }
export function probe() { t\u0061rget = () => 'alternate'; return /* claim */ target(); }
