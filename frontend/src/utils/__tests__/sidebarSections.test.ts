// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A section split across the tab definitions reads as one run, so the rail draws its header once
// ABOUTME: First-seen section order and in-section order are both kept

import { describe, it, expect } from 'vitest';
import { groupBySection } from '../sidebarSections';

const tab = (id: string, section?: string) => ({ id, section });

describe('groupBySection', () => {
  it('joins a section that appears twice into one run where it first appeared', () => {
    const tabs = [
      tab('users', 'Platform'),
      tab('agents', 'Coaching'),
      tab('prompts', 'Configuration'),
      tab('activity', 'Platform'),
      tab('api-tokens', 'Developer'),
      tab('harness-config', 'Configuration'),
    ];
    expect(groupBySection(tabs).map((t) => t.id)).toEqual([
      'users',
      'activity',
      'agents',
      'prompts',
      'harness-config',
      'api-tokens',
    ]);
  });

  it('leaves tabs with no section in their order', () => {
    const tabs = [tab('home'), tab('chat'), tab('settings')];
    expect(groupBySection(tabs)).toEqual(tabs);
  });

  it('gives every section exactly one header', () => {
    const tabs = groupBySection([
      tab('a', 'X'),
      tab('b', 'Y'),
      tab('c', 'X'),
      tab('d', 'Y'),
    ]);
    const headers = tabs.filter((t, i) => i === 0 || t.section !== tabs[i - 1].section);
    expect(headers.map((t) => t.section)).toEqual(['X', 'Y']);
  });
});
