function target() { return 'original'; }
export function probe() { target = () => 'alternate'; return /* claim */ target(); }
