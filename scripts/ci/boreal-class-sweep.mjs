#!/usr/bin/env bun
// ABOUTME: Tailwind class replacement pass for the Boreal Editorial rebrand
// ABOUTME: Rewrites dark-theme-first Pierre utility classes to Boreal tokens

import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { glob } from 'bun';

// Ordered mapping — longer matches first to avoid partial-replace bugs.
const mapping = [
  // Surfaces: dark canvas → light canvas
  ['bg-pierre-dark', 'bg-surface'],
  ['bg-pierre-slate', 'bg-surface-container-low'],
  ['bg-pierre-gray-900', 'bg-surface-container-highest'],
  ['bg-pierre-gray-800', 'bg-surface-container-high'],
  ['bg-pierre-gray-700', 'bg-surface-container'],
  ['bg-pierre-gray-50', 'bg-surface-container-low'],

  // Signature gradient usage
  ['from-pierre-violet via-pierre-cyan to-pierre-activity', 'boreal-hero-gradient'],
  ['from-pierre-violet to-pierre-cyan', 'boreal-hero-gradient'],
  ['bg-gradient-to-r from-pierre-violet to-pierre-cyan', 'boreal-hero-gradient'],
  ['bg-gradient-pierre-horizontal', 'boreal-hero-gradient'],
  ['bg-gradient-pierre', 'boreal-hero-gradient'],

  // Dark-theme text assumptions
  ['text-zinc-400', 'text-on-surface-variant'],
  ['text-zinc-500', 'text-outline'],
  ['text-zinc-300', 'text-on-surface'],
  ['text-zinc-200', 'text-on-surface'],
  ['text-zinc-600', 'text-on-surface-variant'],

  // Border overrides — white/* borders were only visible on dark canvas.
  ['border-white/10', 'ghost-border'],
  ['border-white/5', 'ghost-border'],
  ['border-white/20', 'ghost-border'],

  // Legacy shadow glow classes
  ['shadow-glow-violet', 'shadow-ambient'],
  ['shadow-glow-cyan', 'shadow-ambient'],
  ['shadow-glow-activity', 'shadow-ambient'],
  ['shadow-glow-nutrition', 'shadow-ambient'],
  ['shadow-glow-recovery', 'shadow-ambient'],
  ['shadow-glow-sm', 'shadow-ambient'],
  ['shadow-glow-lg', 'shadow-ambient'],
  ['shadow-glow', 'shadow-ambient'],
];

const files = process.argv.slice(2);

if (files.length === 0) {
  console.error('Usage: bun boreal-class-sweep.mjs <file1> [file2...]');
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

console.log(`\nTotal: ${totalChanges} replacement${totalChanges === 1 ? '' : 's'}`);
