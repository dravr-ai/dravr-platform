// ABOUTME: Tests the Memory pane — facts grouped in a Section per kind, text tabs to filter, the two empty sentences, forget by long-press and by swipe
// ABOUTME: Mocks userApi.listMemoryFacts/forgetMemoryFact and the platform menu; pins the mono relative age and the absence of any pill radius
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { ActionSheetIOS, Alert } from 'react-native';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';

const mockListMemoryFacts = jest.fn();
const mockForgetMemoryFact = jest.fn();

const mockRouterBack = jest.fn();
jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({ back: mockRouterBack, push: jest.fn(), replace: jest.fn() }),
  }),
);

jest.mock('../src/services/api', () => ({
  userApi: {
    listMemoryFacts: (...args: unknown[]) => mockListMemoryFacts(...args),
    forgetMemoryFact: (...args: unknown[]) => mockForgetMemoryFact(...args),
  },
}));

import { MemoryScreen } from '../src/screens/memory/MemoryScreen';
import { networkFailure } from '../integration/app/helpers/apiRefusal';

type Fact = {
  id: string;
  agent_id: string | null;
  agent_title: string | null;
  kind: string;
  predicate_code: string;
  object: string;
  sentence: string;
  confidence: number;
  source_msg_id: string | null;
  updated_at: string;
};

function createFact(overrides: Partial<Fact> = {}): Fact {
  return {
    id: 'fact-1',
    agent_id: null,
    agent_title: null,
    kind: 'goal',
    predicate_code: 'working_toward',
    object: 'sub-3:30 marathon by October',
    sentence: 'You are working toward sub-3:30 marathon by October',
    confidence: 0.85,
    source_msg_id: null,
    updated_at: '2026-04-13T18:00:00Z',
    ...overrides,
  };
}

function renderScreen(): ReturnType<typeof render> {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryScreen />
    </QueryClientProvider>,
  );
}

type Json = { type?: string; props?: Record<string, unknown>; children?: Json[] | null } | string | null;

/** Every `style` object in the rendered tree, flattened, so a radius can be found wherever it landed. */
function allStyles(node: Json | Json[]): Array<Record<string, unknown>> {
  if (node === null || typeof node === 'string') return [];
  if (Array.isArray(node)) return node.flatMap(allStyles);
  const own = ([] as Array<Record<string, unknown>>).concat(
    (node.props?.style as Array<Record<string, unknown>> | Record<string, unknown> | undefined) ?? [],
  );
  return [...own.filter((s) => s && typeof s === 'object'), ...allStyles(node.children ?? [])];
}

/** Every `className` in the rendered tree — walked, not stringified, since the refresh control's props are circular. */
function allClassNames(node: Json | Json[]): string[] {
  if (node === null || typeof node === 'string') return [];
  if (Array.isArray(node)) return node.flatMap(allClassNames);
  const own = typeof node.props?.className === 'string' ? [node.props.className] : [];
  return [...own, ...allClassNames(node.children ?? [])];
}

/** The rows the platform sheet last offered, and a way to pick one by index. */
function presentedSheet() {
  const spy = ActionSheetIOS.showActionSheetWithOptions as unknown as jest.Mock;
  expect(spy).toHaveBeenCalled();
  const [, callback] = spy.mock.calls[spy.mock.calls.length - 1] as [unknown, (index: number) => void];
  return { pick: callback };
}

/** An `Alert.alert` that taps the destructive button the way a confirming athlete would. */
function confirmDestructiveAlerts(): jest.SpyInstance {
  return jest.spyOn(Alert, 'alert').mockImplementation((_title, _msg, buttons) => {
    const confirm = (buttons ?? []).find((b) => b.style === 'destructive');
    confirm?.onPress?.();
  });
}

describe('MemoryScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('renders the empty state when no facts exist', async () => {
    mockListMemoryFacts.mockResolvedValueOnce({ facts: [], total: 0 });
    const { getByText } = renderScreen();
    await waitFor(() => {
      expect(getByText(/No facts stored yet/i)).toBeTruthy();
    });
  });

  it('names the coach a fact belongs to by title, never by id', async () => {
    mockListMemoryFacts.mockResolvedValueOnce({
      facts: [
        createFact({
          agent_id: '7c1f7d2e-4b0a-4f0e-9d3a-0f6c2b8e9a11',
          agent_title: 'Coach Marie',
        }),
      ],
      total: 1,
    });
    const { findByText, queryByText } = renderScreen();
    expect(await findByText(/Coach Marie/)).toBeTruthy();
    expect(queryByText(/7c1f7d2e/)).toBeNull();
  });

  it('renders facts grouped in a Section per kind, with the count on the trailing side', async () => {
    mockListMemoryFacts.mockResolvedValueOnce({
      facts: [
        createFact(),
        createFact({
          id: 'fact-2',
          kind: 'injury',
          predicate_code: 'have',
          object: 'left achilles tendinitis',
          sentence: 'You have left achilles tendinitis',
        }),
      ],
      total: 2,
    });
    const { getByText, getAllByText, getByTestId, getAllByTestId } = renderScreen();
    await waitFor(() => {
      expect(getByText(/sub-3:30 marathon/i)).toBeTruthy();
    });
    expect(getByText(/left achilles tendinitis/i)).toBeTruthy();
    // "Goal" and "Injury" appear twice each — once as a tab and once as a
    // section title — so the group is found by its id and the label by count.
    expect(getByTestId('memory-section-goal')).toBeTruthy();
    expect(getByTestId('memory-section-injury')).toBeTruthy();
    expect(getAllByText('Goal').length).toBeGreaterThan(0);
    expect(getAllByText('Injury').length).toBeGreaterThan(0);
    const counts = getAllByTestId('memory-fact-count');
    expect(counts).toHaveLength(2);
    expect(counts[0].props.children).toEqual(i18n.t('shell.memoryFactCountOne', { count: 1 }));
    expect(counts[0].props.className).toContain('font-mono');
  });

  it('shows the sentence the server rendered, verbatim and in the athlete\'s language', async () => {
    // The sentence is rendered on the server in the athlete's locale; the
    // screen shows it as-is and carries no grammar of its own, so a French
    // athlete's goal reads as French even under English chrome.
    mockListMemoryFacts.mockResolvedValueOnce({
      facts: [
        createFact({
          id: 'fact-fr',
          kind: 'goal',
          predicate_code: 'training_for',
          object: 'un ultra de 26 km au Mont Albert',
          sentence: "Tu t'entraînes pour un ultra de 26 km au Mont Albert",
        }),
      ],
      total: 1,
    });
    const { queryByText, getByText } = renderScreen();
    await waitFor(() => {
      expect(getByText("Tu t'entraînes pour un ultra de 26 km au Mont Albert")).toBeTruthy();
    });
    expect(queryByText(/training_for/)).toBeNull();
    expect(queryByText(/You are/)).toBeNull();
  });

  it('prints how long ago the fact was updated, in mono, as the row\'s only meta', async () => {
    const twoDaysAgo = new Date(Date.now() - 2 * 86_400_000).toISOString();
    mockListMemoryFacts.mockResolvedValueOnce({
      facts: [createFact({ updated_at: twoDaysAgo })],
      total: 1,
    });
    const { getByTestId, queryByText } = renderScreen();
    await waitFor(() => expect(getByTestId('memory-fact-meta')).toBeTruthy());
    const age = getByTestId('memory-fact-meta');
    expect(age.props.children).toEqual(i18n.t('notifications.daysAgo', { count: 2 }));
    expect(age.props.className).toContain('font-mono');
    expect(age.props.className).toContain('tabular-nums');
    // The confidence line left the row with the card.
    expect(queryByText(/Confidence/)).toBeNull();
  });

  it('draws no pill anywhere — no full radius on a tab, a row or the empty state', async () => {
    mockListMemoryFacts.mockResolvedValue({ facts: [createFact()], total: 1 });
    const { toJSON, getByTestId } = renderScreen();
    await waitFor(() => expect(getByTestId('memory-section-goal')).toBeTruthy());
    const tree = toJSON() as Json;
    expect(allClassNames(tree).some((name) => name.includes('rounded-full'))).toBe(false);
    const radii = allStyles(tree)
      .map((s) => s.borderRadius)
      .filter((r): r is number => typeof r === 'number');
    expect(radii.every((r) => r < 999)).toBe(true);
  });

  // The query is filtered server-side, so an empty result under a tab is "none
  // of this type". The unfiltered card told an athlete who has memory that they
  // have none, which is the same lie the web panel told.
  it('says nothing of this type — not nothing at all — when a kind tab is selected', async () => {
    mockListMemoryFacts.mockResolvedValue({ facts: [], total: 0 });
    const { getByTestId, queryByTestId, getByText } = renderScreen();
    await waitFor(() => expect(getByTestId('memory-empty')).toBeTruthy());
    const neverHadAny = i18n.t('shell.memoryEmpty');
    expect(getByText(neverHadAny)).toBeTruthy();
    expect(queryByTestId('memory-show-all-kinds')).toBeNull();

    expect(getByTestId('memory-kind-tab-all').props.accessibilityState).toEqual({ selected: true });
    fireEvent.press(getByTestId('memory-kind-tab-injury'));

    await waitFor(() => expect(getByTestId('memory-empty-filtered')).toBeTruthy());
    expect(queryByTestId('memory-empty')).toBeNull();
    expect(getByTestId('memory-kind-tab-injury').props.accessibilityState).toEqual({ selected: true });
    const filtered = i18n.t('shell.memoryEmptyFiltered');
    expect(filtered).not.toEqual(neverHadAny);
    expect(getByText(filtered)).toBeTruthy();
    expect(mockListMemoryFacts).toHaveBeenLastCalledWith(expect.objectContaining({ kind: 'injury' }));
  });

  it('offers the way back to all types from the filtered empty state', async () => {
    mockListMemoryFacts.mockResolvedValue({ facts: [], total: 0 });
    const { getByTestId } = renderScreen();
    await waitFor(() => expect(getByTestId('memory-empty')).toBeTruthy());

    fireEvent.press(getByTestId('memory-kind-tab-injury'));
    await waitFor(() => expect(getByTestId('memory-show-all-kinds')).toBeTruthy());

    fireEvent.press(getByTestId('memory-show-all-kinds'));

    await waitFor(() => expect(getByTestId('memory-empty')).toBeTruthy());
    expect(mockListMemoryFacts).toHaveBeenLastCalledWith(
      expect.objectContaining({ kind: undefined }),
    );
  });

  it('forgets a fact from the long-press menu, through the confirm', async () => {
    mockListMemoryFacts.mockResolvedValue({ facts: [createFact()], total: 1 });
    mockForgetMemoryFact.mockResolvedValueOnce({ deleted: true });
    jest.spyOn(ActionSheetIOS, 'showActionSheetWithOptions').mockImplementation(() => undefined);
    confirmDestructiveAlerts();

    const { getByLabelText, queryByRole } = renderScreen();
    // The accessibility label names the fact by the server's sentence.
    await waitFor(() => {
      expect(getByLabelText(/Forget You are working toward sub-3:30 marathon/i)).toBeTruthy();
    });
    // Nothing on the row is a button of its own: the trash tile is gone.
    expect(queryByRole('button', { name: /^Forget$/ })).toBeNull();

    fireEvent(getByLabelText(/Forget You are working toward sub-3:30 marathon/i), 'longPress');
    presentedSheet().pick(0);

    await waitFor(() => {
      expect(mockForgetMemoryFact).toHaveBeenCalledWith('fact-1');
    });
    expect(Alert.alert).toHaveBeenCalledWith(
      i18n.t('app.forgetThisFactQ'),
      i18n.t('app.confirmForgetFact', { fact: createFact().sentence }),
      expect.any(Array),
    );
  });

  it('forgets a fact from the swipe action too, through the same confirm', async () => {
    mockListMemoryFacts.mockResolvedValue({ facts: [createFact()], total: 1 });
    mockForgetMemoryFact.mockResolvedValueOnce({ deleted: true });
    confirmDestructiveAlerts();

    const { getByTestId } = renderScreen();
    await waitFor(() => expect(getByTestId('memory-fact-fact-1-swipe-action-forget')).toBeTruthy());

    fireEvent.press(getByTestId('memory-fact-fact-1-swipe-action-forget'));

    await waitFor(() => {
      expect(mockForgetMemoryFact).toHaveBeenCalledWith('fact-1');
    });
    expect(Alert.alert).toHaveBeenCalledWith(
      i18n.t('app.forgetThisFactQ'),
      expect.any(String),
      expect.any(Array),
    );
  });

  it('says why the list failed and offers a retry that asks again', async () => {
    mockListMemoryFacts.mockRejectedValueOnce(networkFailure());
    mockListMemoryFacts.mockResolvedValueOnce({ facts: [], total: 0 });
    const { getByTestId, getByText } = renderScreen();
    await waitFor(() => expect(getByTestId('memory-retry')).toBeTruthy());
    expect(
      getByText(i18n.t('app.failedLoadMemoryFacts', { reason: i18n.t('errors.network') })),
    ).toBeTruthy();

    fireEvent.press(getByTestId('memory-retry'));

    await waitFor(() => expect(getByTestId('memory-empty')).toBeTruthy());
    expect(mockListMemoryFacts).toHaveBeenCalledTimes(2);
  });

  it('draws no back button of its own — the native header carries the way back', async () => {
    // Memory is presented over the tabs; the system header names it and
    // carries Close, so a control drawn here would be a second header idiom.
    mockListMemoryFacts.mockResolvedValue({ facts: [], total: 0 });
    const { getByTestId, queryByTestId } = renderScreen();
    await waitFor(() => {
      expect(getByTestId('memory-screen')).toBeTruthy();
    });
    expect(queryByTestId('back-button')).toBeNull();
    expect(mockRouterBack).not.toHaveBeenCalled();
  });
});
