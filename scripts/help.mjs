// `make help`: lists Makefile targets that carry a `## description` comment.
import { readFileSync } from 'node:fs';

const makefile = readFileSync(process.argv[2] ?? 'Makefile', 'utf8');
const targets = [];
for (const line of makefile.split(/\r?\n/)) {
  const match = /^([a-zA-Z0-9_-]+):.*?## (.*)$/.exec(line);
  if (match) targets.push([match[1], match[2]]);
}
const width = Math.max(...targets.map(([name]) => name.length));
console.info('Usage: make <target>\n');
for (const [name, description] of targets) console.info(`  ${name.padEnd(width)}  ${description}`);
