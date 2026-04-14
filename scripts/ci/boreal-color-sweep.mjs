#!/usr/bin/env bun
// ABOUTME: One-shot color replacement pass for the Boreal Editorial rebrand
// ABOUTME: Rewrites hardcoded Pierre hex literals and rgba() values to Boreal tokens

import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const mapping = [
  // Primary (violet → deep forest)
  ['#8B5CF6', '#00241a'],
  ['#8b5cf6', '#00241a'],
  ['#7C3AED', '#00241a'],
  ['#7c3aed', '#00241a'],

  // Primary container (cyan → primary_container)
  ['#22D3EE', '#0d3b2e'],
  ['#22d3ee', '#0d3b2e'],
  ['#06B6D4', '#0d3b2e'],
  ['#06b6d4', '#0d3b2e'],
  ['#67E8F9', '#234e40'],
  ['#67e8f9', '#234e40'],

  // Activity pillar (emerald → sage)
  ['#4ADE80', '#3c6658'],
  ['#4ade80', '#3c6658'],
  ['#10B981', '#3c6658'],
  ['#10b981', '#3c6658'],
  ['#22C55E', '#234e40'],
  ['#22c55e', '#234e40'],
  ['#059669', '#234e40'],
  ['#86EFAC', '#5e8a78'],
  ['#86efac', '#5e8a78'],

  // Nutrition pillar (amber → warm bronze)
  ['#F59E0B', '#8f6a2e'],
  ['#f59e0b', '#8f6a2e'],
  ['#D97706', '#6e5020'],
  ['#d97706', '#6e5020'],
  ['#FBBF24', '#a87d37'],
  ['#fbbf24', '#a87d37'],
  ['#FCD34D', '#b98a47'],
  ['#fcd34d', '#b98a47'],

  // Recovery pillar (indigo → muted slate)
  ['#818CF8', '#5e7a82'],
  ['#818cf8', '#5e7a82'],
  ['#6366F1', '#5e7a82'],
  ['#6366f1', '#5e7a82'],
  ['#4F46E5', '#425962'],
  ['#4f46e5', '#425962'],

  // Mobility pillar (pink → aged rose)
  ['#EC4899', '#7a4d5e'],
  ['#ec4899', '#7a4d5e'],
  ['#DB2777', '#5a3744'],
  ['#db2777', '#5a3744'],
  ['#F472B6', '#9a6b7b'],
  ['#f472b6', '#9a6b7b'],

  // rgba versions of the same — only the most common alpha values
  ['rgba(139, 92, 246,', 'rgba(0, 36, 26,'],
  ['rgba(124, 58, 237,', 'rgba(0, 36, 26,'],
  ['rgba(34, 211, 238,', 'rgba(13, 59, 46,'],
  ['rgba(6, 182, 212,', 'rgba(13, 59, 46,'],
  ['rgba(74, 222, 128,', 'rgba(60, 102, 88,'],
  ['rgba(16, 185, 129,', 'rgba(60, 102, 88,'],
  ['rgba(34, 197, 94,', 'rgba(35, 78, 64,'],
  ['rgba(245, 158, 11,', 'rgba(143, 106, 46,'],
  ['rgba(217, 119, 6,', 'rgba(110, 80, 32,'],
  ['rgba(251, 191, 36,', 'rgba(168, 125, 55,'],
  ['rgba(129, 140, 248,', 'rgba(94, 122, 130,'],
  ['rgba(99, 102, 241,', 'rgba(94, 122, 130,'],
  ['rgba(79, 70, 229,', 'rgba(66, 89, 98,'],
  ['rgba(236, 72, 153,', 'rgba(122, 77, 94,'],
  ['rgba(219, 39, 119,', 'rgba(90, 55, 68,'],

  // Pierre dark surfaces → boreal light surfaces
  ['#0F0F1A', '#f9f9f6'],
  ['#0f0f1a', '#f9f9f6'],
  ['#1E1E2E', '#f4f4f1'],
  ['#1e1e2e', '#f4f4f1'],
  ['rgba(15, 15, 26,', 'rgba(26, 28, 27,'],
  ['rgba(30, 30, 46,', 'rgba(244, 244, 241,'],
];

const files = process.argv.slice(2);

if (files.length === 0) {
  console.error('Usage: bun boreal-color-sweep.mjs <file1> [file2...]');
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
    console.log(`${relPath}: ${fileChanges} replacement${fileChanges === 1 ? '' : 's'}`);
    totalChanges += fileChanges;
  }
}

console.log(`\nTotal: ${totalChanges} replacement${totalChanges === 1 ? '' : 's'} across ${files.length} file${files.length === 1 ? '' : 's'}`);
