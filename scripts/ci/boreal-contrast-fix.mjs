#!/usr/bin/env bun
// ABOUTME: Fix dark-on-dark contrast regressions from the text-white sweep
// ABOUTME: Where a dark-colored background pairs with text-on-surface (dark ink),
//          flip the text to text-on-primary (white) or text-on-error to restore
//          WCAG AA contrast.

import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

// Dark-colored backgrounds where the text should be on-primary (white) not on-surface (ink).
const darkBgs = [
  'pierre-violet',
  'pierre-cyan',
  'pierre-activity',
  'pierre-nutrition',
  'pierre-recovery',
  'pierre-mobility',
  'pierre-green-500',
  'pierre-green-600',
  'pierre-green-700',
  'pierre-blue-500',
  'pierre-blue-600',
  'pierre-blue-700',
  'pierre-yellow-500',
  'pierre-yellow-600',
  'pierre-yellow-700',
  'pierre-red-500',
  'pierre-red-600',
  'pierre-red-700',
  'pierre-purple-500',
  'pierre-purple-600',
  'pierre-purple-700',
  'primary',
  'primary-container',
];

// Only flip when the background is SOLID (no /NN opacity suffix). A low-opacity
// variant like bg-pierre-activity/20 renders as a light tint over the surface,
// so dark text still reads well on it.
const patterns = [];

for (const bg of darkBgs) {
  // `bg-X text-on-surface` in either order, avoiding text-on-surface-variant.
  patterns.push([
    new RegExp(`\\bbg-${bg}(\\s+[^"'\\n]*?)text-on-surface\\b(?!-)(?!/)`, 'g'),
    (match, middle = '') => `bg-${bg}${middle}text-on-primary`,
  ]);
  patterns.push([
    new RegExp(`\\btext-on-surface\\b(?!-)([^"'\\n]*?)\\s+bg-${bg}(?!/)`, 'g'),
    (match, middle = '') => `text-on-primary${middle} bg-${bg}`,
  ]);
  // Edge: same attribute, immediately adjacent
  patterns.push([
    new RegExp(`\\bbg-${bg} text-on-surface\\b(?!-)(?!/)`, 'g'),
    () => `bg-${bg} text-on-primary`,
  ]);
}

// bg-error → text-on-error
patterns.push([
  /\bbg-error(\s+[^"'\n]*?)text-on-surface\b(?!-)(?!\/)/g,
  (match, middle = '') => `bg-error${middle}text-on-error`,
]);
patterns.push([
  /\bbg-error text-on-surface\b(?!-)(?!\/)/g,
  () => `bg-error text-on-error`,
]);

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error('Usage: bun boreal-contrast-fix.mjs <file1> [file2...]');
  process.exit(1);
}

let totalChanges = 0;
for (const relPath of files) {
  const path = resolve(relPath);
  const before = readFileSync(path, 'utf8');
  let after = before;
  let fileChanges = 0;
  for (const [re, replacer] of patterns) {
    after = after.replace(re, (...args) => {
      fileChanges++;
      return replacer(...args);
    });
  }
  if (fileChanges > 0) {
    writeFileSync(path, after, 'utf8');
    console.log(`${relPath}: ${fileChanges}`);
    totalChanges += fileChanges;
  }
}

console.log(`\nTotal: ${totalChanges} contrast fix${totalChanges === 1 ? '' : 'es'}`);
