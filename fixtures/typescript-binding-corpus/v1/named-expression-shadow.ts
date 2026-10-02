function target() { return 'original'; }
const invoke = function target() { return false ? /* claim */ target() : 'named'; };
export function probe() { return invoke(); }
