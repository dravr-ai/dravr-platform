#!/usr/bin/env bun
// ABOUTME: Text-color replacement pass for the Boreal rebrand
// ABOUTME: Replaces dark-theme text-white with text-on-surface; gradient/primary
//          contexts are handled manually afterwards.

import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const mapping = [
  // Hover states — almost always on light surface now
  ['hover:text-white', 'hover:text-on-surface'],
  ['hover:bg-white/5', 'hover:bg-surface-container-low'],
  ['hover:bg-white/10', 'hover:bg-surface-container'],

  // Generic text-white — blanket swap. Legit cases (text on primary buttons,
  // boreal-overlay cards, gradient avatars) get patched by hand afterwards.
  ['text-white', 'text-on-surface'],

  // Placeholder / bg-white remnants
  ['placeholder-zinc-500', 'placeholder:text-outline'],
  ['placeholder-zinc-400', 'placeholder:text-on-surface-variant'],
  ['bg-white/10', 'bg-surface-container-high'],
  ['bg-white/5', 'bg-surface-container-low'],
  ['bg-white/20', 'bg-surface-container-highest'],
];

const files = process.argv.slice(2);

if (files.length === 0) {
  console.error('Usage: bun boreal-text-sweep.mjs <file1> [file2...]');
  process.exit(1);
}

let totalChanges = 0;
for (const relPath of files) {
  const path = resolve(relPath);
  const before = readFileSync(path, 'utf8');
  let after = before;
  let fileChanges = 0;
  for (const [from, to] of mapping) {
    const parts = after.split(from);
    if (parts.length > 1) {
      fileChanges += parts.length - 1;
      after = parts.join(to);
    }
  }
  if (fileChanges > 0) {
    writeFileSync(path, after, 'utf8');
    console.log(`${relPath}: ${fileChanges}`);
    totalChanges += fileChanges;
  }
}

console.log(`\nTotal: ${totalChanges}`);
