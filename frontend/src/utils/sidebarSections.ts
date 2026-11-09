// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Orders sidebar tabs so each section is one contiguous run under a single header
// ABOUTME: Sections keep the order they first appear in, and tabs keep their order inside a section

/**
 * The rail starts a header wherever a tab's section differs from the one above
 * it, so a section split across the definition list (super-admin tabs appended
 * after the developer ones, say) drew its header twice. Grouping here keeps the
 * definitions free to be written in any order.
 */
export function groupBySection<T extends { section?: string }>(tabs: readonly T[]): T[] {
  const runs = new Map<string, T[]>();
  for (const tab of tabs) {
    const key = tab.section ?? '';
    const run = runs.get(key);
    if (run) {
      run.push(tab);
    } else {
      runs.set(key, [tab]);
    }
  }
  return [...runs.values()].flat();
}
