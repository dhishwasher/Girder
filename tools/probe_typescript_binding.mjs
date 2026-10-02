import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const source = readFileSync(process.argv[2], 'utf8');
const mode = process.argv[3];
let result;
if (mode === 'module') {
  const module = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
  result = await module.probe();
} else if (mode === 'script') {
  result = runInNewContext(`${source}\n;probe()`, {}, { timeout: 1000 });
} else {
  throw new Error(`Unsupported runtime mode: ${mode}`);
}
console.log(JSON.stringify(result));
