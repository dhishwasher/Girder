declare const decorate: any;
@decorate class Box {}
function target() { return 'original'; }
export function probe() { return /* claim */ target(); }
