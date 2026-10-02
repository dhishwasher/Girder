const \u0078 = 1;
function target() { return 'original'; }
export function probe() { return /* claim */ target(); }
