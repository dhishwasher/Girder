const holder = { run() { function target() { return 'original'; } return /* claim */ target(); } };
export function probe() { return holder.run(); }
