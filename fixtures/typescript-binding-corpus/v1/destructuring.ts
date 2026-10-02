function target() { return 'original'; }
export function probe() { const replacement = { ['target']: () => 'alternate' }; ({target} = replacement); return /* claim */ target(); }
