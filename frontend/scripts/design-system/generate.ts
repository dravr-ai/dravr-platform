// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Rebuilds the Dravr Boreal Design System artifact's files into frontend/design-system/dist/project
// ABOUTME: Tokens from the shared sources, the web primitives as one IIFE bundle, and the app's compiled Tailwind sheet

import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import type { BunPlugin } from 'bun';

import { BUNDLE_ENTRY, CONTENT_ROOT, FRONTEND_ROOT, buildTokens, componentSources } from './sources';

const REPO = 'dravr-ai/dravr-platform';
const OUT = resolve(CONTENT_ROOT, 'dist');
const PROJECT = resolve(OUT, 'project');
const NAMESPACE = 'Boreal';
/** The React the artifact's previews load from jsDelivr; the bundle reads it from window. */
const REACT_MAJOR = '18';
const FONTS_IMPORT =
  "@import url('https://fonts.googleapis.com/css2?family=Schibsted+Grotesk:wght@400;500;600;700&family=Plus+Jakarta+Sans:wght@400;500;600;700&family=Newsreader:ital,opsz,wght@1,6..72,400&family=JetBrains+Mono:wght@400;500&display=swap');";

interface AssetRecord { name: string; blob: string; size: number; type: string }
interface ArtifactRecord {
  artifact: string;
  title: string;
  namespace: string;
  createdOnFiles: { v: number; at: string };
  assetGroups: Record<string, { name: string; tile: string; order: string[]; files: Record<string, AssetRecord> }>;
}

function run(cmd: string[], cwd = FRONTEND_ROOT): string {
  const proc = Bun.spawnSync(cmd, { cwd, stdout: 'pipe', stderr: 'pipe' });
  if (proc.exitCode !== 0) {
    throw new Error(`${cmd.join(' ')} exited ${proc.exitCode}\n${proc.stderr.toString()}`);
  }
  return proc.stdout.toString().trim();
}

/** React from the page's globals, and DravrLogo's /brand/ paths pointed at the artifact's uploaded marks. */
function plugin(marks: Record<'ink' | 'mint', Record<string, string>>): BunPlugin {
  return {
    name: 'design-system-globals',
    setup(build) {
      build.onResolve({ filter: /^react(-dom)?(\/.*)?$/ }, (args) => ({ path: args.path, namespace: 'react-global' }));
      build.onLoad({ filter: /.*/, namespace: 'react-global' }, (args) => {
        if (args.path.startsWith('react/jsx')) {
          return {
            loader: 'js',
            contents:
              'const R=window.React;const j=(t,p,k)=>R.createElement(t,k===undefined?p:{...p,key:k});' +
              'export const jsx=j,jsxs=j,jsxDEV=j;export const Fragment=R.Fragment;',
          };
        }
        const global = args.path.startsWith('react-dom') ? 'ReactDOM' : 'React';
        return { loader: 'js', contents: `const m=window.${global};export default m;export const {${Object.keys(global === 'React' ? REACT_NAMED : { createPortal: 1 }).join(',')}}=m;` };
      });
      build.onLoad({ filter: /DravrLogo\.tsx$/ }, async (args) => {
        const source = await Bun.file(args.path).text();
        const rewritten = source
          .replace('`/brand/mark-ink-${edge}.png`', '`/_blob/${MARKS.ink[edge]}`')
          .replace('`/brand/mark-mint-${edge}.png`', '`/_blob/${MARKS.mint[edge]}`');
        if (!rewritten.includes('MARKS.ink') || !rewritten.includes('MARKS.mint')) {
          throw new Error('DravrLogo.tsx: the /brand/ asset paths moved; the bundle cannot point them at the artifact');
        }
        return { loader: 'tsx', contents: `const MARKS=${JSON.stringify(marks)};\n${rewritten}` };
      });
    },
  };
}

/** The React named exports the bundled components import. */
const REACT_NAMED = {
  Children: 1, Fragment: 1, cloneElement: 1, createContext: 1, createElement: 1, forwardRef: 1, isValidElement: 1,
  memo: 1, useCallback: 1, useContext: 1, useEffect: 1, useId: 1, useLayoutEffect: 1, useMemo: 1, useRef: 1, useState: 1,
};

function markIds(record: ArtifactRecord): Record<'ink' | 'mint', Record<string, string>> {
  const files = record.assetGroups.Logos?.files ?? {};
  const marks: Record<'ink' | 'mint', Record<string, string>> = { ink: {}, mint: {} };
  for (const f of Object.values(files)) {
    const m = /^mark-(ink|mint)-(\d+)\.png$/.exec(f.name);
    if (m) marks[m[1] as 'ink' | 'mint'][m[2]] = f.blob;
  }
  if (Object.keys(marks.ink).length === 0 || Object.keys(marks.mint).length === 0) {
    throw new Error('artifact.json records no mark-ink-*/mark-mint-* uploads');
  }
  return marks;
}

async function buildBundle(cards: string[], marks: Record<'ink' | 'mint', Record<string, string>>): Promise<string> {
  const result = await Bun.build({
    entrypoints: [BUNDLE_ENTRY],
    format: 'iife',
    minify: true,
    target: 'browser',
    plugins: [plugin(marks)],
    define: { 'process.env.NODE_ENV': '"production"' },
  });
  if (!result.success || result.outputs.length !== 1) {
    throw new Error(`bundle build failed:\n${result.logs.map(String).join('\n')}`);
  }
  const code = await result.outputs[0].text();
  if (/<\/script|<!--/i.test(code)) throw new Error('bundle.js contains </script or <!--, which would break an inlined copy');
  if (/\bimport\s*\(|\beval\(/.test(code)) throw new Error('bundle.js contains a dynamic import or eval');
  const header = { format: 4, namespace: NAMESPACE, components: cards.map((name) => ({ name })) };
  return `/* @ds-bundle: ${JSON.stringify(header)} */\n${code}`;
}

/** The app's own stylesheet, purged to the bundled components and the previews, dark driven by data-theme. */
function buildStylesheet(): string {
  const config = resolve(OUT, 'tailwind.design-system.cjs');
  const css = resolve(OUT, 'tailwind.css');
  writeFileSync(
    config,
    `const base = require(${JSON.stringify(resolve(FRONTEND_ROOT, 'tailwind.config.cjs'))});\n` +
      `module.exports = { ...base, content: ${JSON.stringify([
        resolve(FRONTEND_ROOT, 'src/components/ui/*.tsx'),
        resolve(FRONTEND_ROOT, 'src/components/chat/MessageBubble.tsx'),
        resolve(FRONTEND_ROOT, 'src/components/DravrLogo.tsx'),
        resolve(CONTENT_ROOT, 'components/*/preview.html'),
      ])} };\n`,
  );
  run([resolve(FRONTEND_ROOT, '../node_modules/.bin/tailwindcss'), '-c', config, '-i', 'src/index.css', '-o', css, '--minify']);
  let sheet = readFileSync(css, 'utf8');
  for (const marker of ['.btn-primary', '.chat-bubble-user', '--color-primary']) {
    if (!sheet.includes(marker)) throw new Error(`compiled stylesheet has no ${marker}; the Tailwind run produced the wrong sheet`);
  }
  sheet = sheet.replaceAll('html.dark', '[data-theme="dark"]').replace(/(?<![\w\\:-])\.dark(?=[\s*),{])/g, '[data-theme="dark"]');
  if (/<\/style/i.test(sheet)) throw new Error('bundle.css contains </style');
  return [
    FONTS_IMPORT,
    "/* ABOUTME: The web app's compiled stylesheet (frontend/src/index.css + tailwind.config.cjs), purged to the bundled components */",
    '/* ABOUTME: html.dark and .dark are rewritten to [data-theme="dark"] so the artifact\'s theme switch drives the same tokens */',
    sheet,
  ].join('\n');
}

function copyTree(from: string, to: string): void {
  for (const name of readdirSync(from)) {
    const src = join(from, name);
    const dst = join(to, name);
    if (statSync(src).isDirectory()) {
      mkdirSync(dst, { recursive: true });
      copyTree(src, dst);
    } else {
      cpSync(src, dst);
    }
  }
}

function listFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    return statSync(p).isDirectory() ? listFiles(p) : [p];
  });
}

async function main(): Promise<void> {
  const record = JSON.parse(readFileSync(resolve(CONTENT_ROOT, 'artifact.json'), 'utf8')) as ArtifactRecord;
  const sha = run(['git', 'rev-parse', '--short', 'HEAD']);
  const branch = run(['git', 'rev-parse', '--abbrev-ref', 'HEAD']);
  const author = run(['git', 'log', '-1', '--format=%an']);
  const now = new Date().toISOString().replace(/\.\d{3}Z$/, 'Z');

  if (existsSync(OUT)) rmSync(OUT, { recursive: true });
  mkdirSync(resolve(PROJECT, 'components'), { recursive: true });

  const sources = componentSources();
  const cards = Object.keys(sources);
  const tokens = buildTokens({ repo: REPO, ref: `${branch}@${sha}` }, sources, now.slice(0, 10));

  writeFileSync(resolve(PROJECT, 'tokens.json'), `${JSON.stringify(tokens, null, 1)}\n`);
  cpSync(resolve(CONTENT_ROOT, 'brand-book.md'), resolve(PROJECT, 'README.md'));
  copyTree(resolve(CONTENT_ROOT, 'components'), resolve(PROJECT, 'components'));
  mkdirSync(resolve(PROJECT, 'assets'), { recursive: true });
  copyTree(resolve(CONTENT_ROOT, 'assets'), resolve(PROJECT, 'assets'));
  writeFileSync(resolve(PROJECT, 'components/bundle.js'), await buildBundle(cards, markIds(record)));
  writeFileSync(resolve(PROJECT, 'components/bundle.css'), buildStylesheet());

  const index = {
    v: 3,
    layout: 'files',
    createdOnFiles: record.createdOnFiles,
    title: record.title,
    namespace: record.namespace,
    libraries: [{ name: 'react', version: REACT_MAJOR }, { name: 'react-dom', version: REACT_MAJOR }],
    sections: {},
    groups: Object.keys(record.assetGroups),
    assetGroups: record.assetGroups,
    blobs: {},
    docs: { readme: 'project/README.md', sections: [] },
    lastChange: { by: author, at: now, via: `GitHub · ${REPO}@${sha}`, note: `Regenerated from ${branch}@${sha}` },
  };
  writeFileSync(resolve(PROJECT, 'design-system.json'), `${JSON.stringify(index, null, 1)}\n`);

  const files = listFiles(PROJECT).map((f) => relative(OUT, f)).sort();
  console.log(`Design system for ${record.artifact} written to ${relative(FRONTEND_ROOT, OUT)}/ (${files.length} files):`);
  for (const f of files) console.log(`  ${f}`);
}

await main();
