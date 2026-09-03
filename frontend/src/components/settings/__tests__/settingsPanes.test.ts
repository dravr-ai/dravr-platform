// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Asserts web's settings tab table is derived from the shared pane declaration, not typed twice
// ABOUTME: Web had ten panes and the phone one scroll, with nothing failing when the two disagreed

import { describe, it, expect } from 'vitest';
import { SETTINGS_PANES, settingsPanesFor, settingsPaneSections } from '@pierre/shared-constants';
import { SETTINGS_TABS } from '../settingsTabs';

describe('settings pane parity — web', () => {
  it('lists exactly the panes the declaration serves web, in declaration order', () => {
    const declared = settingsPanesFor('web');
    expect(SETTINGS_TABS.map((tab) => tab.id)).toEqual(declared.map((pane) => pane.id));
    expect(SETTINGS_TABS.map((tab) => tab.nameKey)).toEqual(declared.map((pane) => pane.nameKey));
    expect(SETTINGS_TABS.map((tab) => tab.hintKey)).toEqual(declared.map((pane) => pane.hintKey));
  });

  it('gives every pane an icon', () => {
    // The icons are the one thing that cannot live in the shared declaration.
    // A pane added there with no glyph here renders a hole in the rail.
    expect(SETTINGS_TABS.filter((tab) => tab.icon === undefined)).toEqual([]);
  });

  it('offers no AI-provider pane', () => {
    // The per-athlete provider pane asked for Gemini, Groq, Cohere and local
    // keys, stored them, and changed nothing about the coaching that followed.
    expect(SETTINGS_TABS.map((tab) => String(tab.id))).not.toContain('llm');
    expect(SETTINGS_PANES.map((pane) => String(pane.id))).not.toContain('llm');
    expect(SETTINGS_PANES.some((pane) => pane.nameKey === 'settingsTabs.ai')).toBe(false);
  });

  it('has a mobile counterpart for every pane it serves', () => {
    // The parity assertion in one direction; the mobile suite makes the same
    // check against the same declaration from its own side.
    const webOnly = settingsPanesFor('web').filter((pane) => pane.mobile === null);
    expect(webOnly.map((pane) => pane.id)).toEqual([]);
  });

  it('groups Account and About the same way on both clients', () => {
    // Usage stood alone on the phone and sat inside Account on web; MCP apps
    // likewise. Both clients now render these two panes from this list.
    expect([...settingsPaneSections('account')]).toEqual([
      'account-status',
      'usage',
      'security',
      'connected-mcp-apps',
      'sign-out',
    ]);
    expect([...settingsPaneSections('about')]).toEqual([
      'version',
      'coach-model',
      'help',
      'legal',
    ]);
  });

  it('leaves a single-destination pane with no section list to disagree about', () => {
    // `holds` is only for panes that group several things. A pane with one
    // destination has nothing to order, and inventing a one-entry list for it
    // would be data neither client reads.
    const grouped = SETTINGS_PANES.filter((pane) => pane.holds !== undefined).map((pane) => pane.id);
    expect(grouped).toEqual(['about', 'account']);
  });
});
