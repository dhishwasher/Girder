function target() { return 'original'; }
function invoke(target) { return /* claim */ target(); }
export function probe() { return invoke(() => 'alternate'); }
