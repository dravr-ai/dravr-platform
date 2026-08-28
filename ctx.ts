import path from 'path'; import fs from 'fs';
import { scanUntranslated } from './frontend/src/i18n/untranslatedScan';
const W = process.cwd();
const ROOTS = [
  path.join(W,'frontend/src/components'), path.join(W,'frontend/src/onboarding'),
  path.join(W,'frontend/src/App.tsx'),
  path.join(W,'frontend-mobile/src'), path.join(W,'frontend-mobile/app'),
];
const hits = scanUntranslated(ROOTS).filter(h => h.scope === 'athlete');
const byFile = new Map<string, Set<string>>();
for (const h of hits) {
  const rel = path.relative(W, h.file);
  if (!byFile.has(rel)) byFile.set(rel, new Set());
  byFile.get(rel)!.add(h.text);
}
const target = process.argv[2];
const out: Record<string, Array<{text:string; line:number; src:string}>> = {};
for (const [rel, set] of byFile) {
  if (target && !rel.includes(target)) continue;
  const lines = fs.readFileSync(path.join(W, rel), 'utf8').split('\n');
  out[rel] = [...set].map(text => {
    const i = lines.findIndex(l => l.includes(text));
    return { text, line: i + 1, src: i >= 0 ? lines[i].trim().slice(0, 150) : '(multiline)' };
  });
}
console.log(JSON.stringify(out, null, 1));
