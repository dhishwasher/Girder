namespace Names { export const n = 1; }
function target() { return 'original'; }
function probe() { return /* claim */ target(); }
