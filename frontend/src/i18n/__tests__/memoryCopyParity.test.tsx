// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the memory screen's title, blurb and empty state to ONE catalogue key each, as the browser renders them
// ABOUTME: They existed twice — shell.* for the browser, app.* for the phone — and the copies had already drifted

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { SUPPORTED_LANGUAGES, defaultI18nConfig, i18n } from '@pierre/i18n';
import { SETTINGS_PANES } from '@pierre/shared-constants';
import MemoryPanel from '../../components/memory/MemoryPanel';

vi.mock('../../services/api', async () => ({
  userApi: {
    listMemoryFacts: vi.fn().mockResolvedValue({ facts: [], total: 0 }),
    forgetMemoryFact: vi.fn(),
  },
}));

/**
 * The one key per string, and what each is for.
 *
 * The title read "Ce que TON coach retient de toi" in the browser and "Ce que
 * LE coach retient de toi" on the phone; the blurb differed by a whole rewrite.
 * Nothing kept them in step, so a wording change landed on one client only.
 *
 * This is the browser's half. The phone's half is the "copy shared with the
 * web panel" block of frontend-mobile/__tests__/MemoryScreen.test.tsx, which
 * renders the screen against the same keys; change this list and that one
 * together. Each half renders its client in French, where the two copies had
 * drifted, and compares what is on screen with the catalogue's own string. A
 * search of both components' source for the key names fails a key passed
 * through a variable and passes a key read but never rendered.
 */
const SHARED_KEYS = {
  title: 'shell.memoryTitle',
  blurb: 'app.memoryPanelBlurb',
  empty: 'shell.memoryEmpty',
  emptyFiltered: 'shell.memoryEmptyFiltered',
  showAllKinds: 'shell.memoryShowAllKinds',
} as const;

/**
 * Read by the browser alone. The phone's empty state is one sentence and, when
 * filtered, one link (`ui/EmptyState`, Boreal v2.2 Phase 4), so it has no hint
 * line to translate; the browser's memory panel still shows one under each
 * sentence. The keys stay in the catalogue for as long as the browser reads them.
 */
const WEB_ONLY_KEYS = {
  emptyHint: 'shell.memoryEmptyHint',
  emptyFilteredHint: 'shell.memoryEmptyFilteredHint',
} as const;

/** The second copies, retired: each said the same thing as a key above. */
const RETIRED_KEYS = [
  'app.whatCoachRemembers',
  'app.memoryBlurb',
  'app.noFactsYet',
  'app.memoryEmptyBlurb',
  // The third copy of the title: the settings pane's own hint said the same
  // sentence, byte for byte in fr/es/de/pt, and the pane now points at
  // `shell.memoryTitle` like both screens do.
  'settingsTabs.memoryHint',
];

function bundleFor(language: string): Record<string, unknown> {
  const resources = defaultI18nConfig.resources as Record<string, { translation: Record<string, unknown> }>;
  return resources[language].translation;
}

function leaf(bundle: Record<string, unknown>, key: string): unknown {
  return key.split('.').reduce<unknown>(
    (node, part) => (node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined),
    bundle,
  );
}

/**
 * The catalogue's own French string for `key`, read from the bundle rather
 * than through `t()`: a missing key makes `t()` answer with the key itself,
 * which a component reading the same missing key would match.
 */
function french(key: string): string {
  const text = leaf(bundleFor('fr'), key);
  expect(typeof text, `fr has no string at ${key}`).toBe('string');
  return text as string;
}

function renderPanel() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryPanel />
    </QueryClientProvider>,
  );
}

describe('memory screen copy parity — web', () => {
  beforeEach(async () => {
    await i18n.changeLanguage('fr');
  });

  afterEach(async () => {
    await i18n.changeLanguage('en');
  });

  it('renders the title, blurb and empty sentence from the shared keys', async () => {
    renderPanel();

    const empty = await screen.findByTestId('memory-empty');
    expect(screen.getByRole('heading', { name: french(SHARED_KEYS.title) })).toBeInTheDocument();
    expect(screen.getByText(french(SHARED_KEYS.blurb))).toBeInTheDocument();
    expect(empty).toHaveTextContent(french(SHARED_KEYS.empty));
    expect(empty).toHaveTextContent(french(WEB_ONLY_KEYS.emptyHint));
  });

  it('renders the filtered empty sentence and the way back from the shared keys', async () => {
    renderPanel();
    await screen.findByTestId('memory-empty');

    fireEvent.click(screen.getByTestId('memory-kind-chip-injury'));

    const filtered = await screen.findByTestId('memory-empty-filtered');
    expect(filtered).toHaveTextContent(french(SHARED_KEYS.emptyFiltered));
    expect(filtered).toHaveTextContent(french(WEB_ONLY_KEYS.emptyFilteredHint));
    expect(screen.getByTestId('memory-show-all-kinds')).toHaveTextContent(french(SHARED_KEYS.showAllKinds));
  });

  it('names the settings pane with the screen title, not a copy of it', () => {
    const pane = SETTINGS_PANES.find((entry) => entry.id === 'memory');
    expect(pane?.hintKey).toBe(SHARED_KEYS.title);
  });

  it('leaves no second copy behind in any locale of the catalogue', () => {
    // A client still reading a retired key would now print the dotted key
    // itself, which the rendered assertions above would refuse.
    for (const language of SUPPORTED_LANGUAGES) {
      for (const key of RETIRED_KEYS) {
        expect({ language, key, value: leaf(bundleFor(language), key) }).toEqual({ language, key, value: undefined });
      }
      for (const key of [...Object.values(SHARED_KEYS), ...Object.values(WEB_ONLY_KEYS)]) {
        expect({ language, key, type: typeof leaf(bundleFor(language), key) }).toEqual({ language, key, type: 'string' });
      }
    }
  });
});
