// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the verdict sheet a reply's chip opens — one card per row, its words, its loading line
// ABOUTME: Studies open as links, the source reply expands on a press, and support's ids sit behind the menu

import React from 'react';
import { Linking } from 'react-native';
import { fireEvent, render, waitFor, within } from '@testing-library/react-native';
import type { ClaimVerdict } from '@pierre/shared-types';
import { VerdictSheet, type VerdictSheetProps, type VerdictSource } from '../src/screens/chat/VerdictSheet';

const mockPresentMenu = jest.fn();
// The actions are the platform's own menu; here it is a mock that hands back
// the rows the card wired into it.
jest.mock('../src/utils/presentMenu', () => ({
  presentMenu: (...args: unknown[]) => mockPresentMenu(...args),
}));

type ClassNamed = { props: { className?: string }; children?: unknown };

/** Every className in a rendered subtree, host views included. */
function classNames(node: unknown): string[] {
  if (!node || typeof node !== 'object') return [];
  const found: string[] = [];
  const el = node as ClassNamed;
  if (el.props && typeof el.props.className === 'string') found.push(el.props.className);
  const children = Array.isArray(node) ? node : el.children;
  if (Array.isArray(children)) {
    for (const child of children) found.push(...classNames(child));
  }
  return found;
}

function row(overrides: Partial<ClaimVerdict> & { id: string }): ClaimVerdict {
  return {
    conversation_id: 'conv-1',
    message_id: 'msg-1',
    agent_id: 'coach-tempo',
    claim_text: 'Your VO2max is 82.',
    category: 'physiological',
    status: 'contradicted',
    evidence_strength: 'none',
    confidence: 0.91,
    layer_fired: 'deterministic',
    explanation: null,
    evidence_refs: null,
    created_at: '2026-08-22T10:00:05Z',
    ...overrides,
  };
}

const FIRST = row({
  id: 'verdict-1',
  explanation: 'A VO2max of 82 sits above every value recorded for a recreational athlete.',
  evidence_refs: 'acsm:2021-vo2max, doi:10.1/abc',
});
const SECOND = row({
  id: 'verdict-2',
  claim_text: 'Six hours of sleep is enough.',
  status: 'unsupported',
  evidence_strength: 'weak',
  confidence: 0.6,
});
const STUDIED = row({
  id: 'verdict-3',
  claim_text: '- If it felt smooth, count the session as a good one.',
  status: 'supported',
  evidence_strength: 'mixed',
  confidence: 0.8,
  explanation: 'Supported by Rønnestad and Mujika 2014',
  evidence_refs: 'doi:10.1111/sms.12104',
});
const SOURCE: VerdictSource = {
  title: 'Easy Tuesday run',
  content: 'No HR zones on file, so this is an estimate.\n\n- If it felt smooth, count the session as a good one.',
  createdAt: '2026-08-22T10:00:04Z',
};

function renderSheet(props: Partial<VerdictSheetProps>) {
  const onClose = jest.fn();
  const onAskAboutClaim = jest.fn();
  const view = render(
    <VerdictSheet
      visible
      verdicts={[]}
      loading={false}
      onClose={onClose}
      onAskAboutClaim={onAskAboutClaim}
      {...props}
    />,
  );
  return { ...view, onClose, onAskAboutClaim };
}

describe('VerdictSheet', () => {
  beforeEach(() => mockPresentMenu.mockClear());

  it('draws one card per verdict row, each naming its claim', () => {
    const { getAllByTestId, getByText } = renderSheet({ verdicts: [FIRST, SECOND] });

    expect(getAllByTestId('verdict-card')).toHaveLength(2);
    expect(getByText('Your VO2max is 82.')).toBeTruthy();
    expect(getByText('Six hours of sleep is enough.')).toBeTruthy();
    expect(getByText('Verdicts on this reply')).toBeTruthy();
  });

  it('spells out the status, the evidence, the confidence, the findings and the references in words', () => {
    const { getByText, queryByText } = renderSheet({ verdicts: [FIRST] });

    expect(getByText('About this claim')).toBeTruthy();
    expect(getByText('contradicted')).toBeTruthy();
    // Evidence and confidence share one tertiary line.
    expect(getByText(/evidence: none.*confidence: 91%/)).toBeTruthy();
    expect(getByText('What the detector found')).toBeTruthy();
    expect(getByText(FIRST.explanation as string)).toBeTruthy();
    expect(getByText('Evidence references')).toBeTruthy();
    expect(getByText('acsm:2021-vo2max')).toBeTruthy();
    expect(getByText('doi:10.1/abc')).toBeTruthy();
    expect(getByText(/^Verdict emitted /)).toBeTruthy();
    expect(queryByText('Loading verdicts…')).toBeNull();
  });

  it('omits the findings and reference sections a row did not carry', () => {
    const { queryByText } = renderSheet({ verdicts: [SECOND] });

    expect(queryByText('What the detector found')).toBeNull();
    expect(queryByText('Evidence references')).toBeNull();
  });

  it('says the verdicts are loading while the read is in flight with nothing to show', () => {
    const { getByText, queryAllByTestId } = renderSheet({ verdicts: [], loading: true });

    expect(getByText('Loading verdicts…')).toBeTruthy();
    expect(queryAllByTestId('verdict-card')).toHaveLength(0);
  });

  it('hands the pressed card\'s own row to the ask action', () => {
    const { getAllByText, onAskAboutClaim } = renderSheet({ verdicts: [FIRST, SECOND] });

    fireEvent.press(getAllByText('Ask me about this claim')[1]);

    expect(onAskAboutClaim).toHaveBeenCalledTimes(1);
    expect(onAskAboutClaim).toHaveBeenCalledWith(SECOND);
  });

  it('closes from the close control', () => {
    const { getByTestId, onClose } = renderSheet({ verdicts: [FIRST] });

    fireEvent.press(getByTestId('verdict-sheet-close'));

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('closes from a press on the scrim', () => {
    const { getByTestId, onClose } = renderSheet({ verdicts: [FIRST] });

    fireEvent.press(getByTestId('verdict-sheet-backdrop'));

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('opens a study at its doi.org page from an inline link', async () => {
    const openURL = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
    const { getByText, getByRole, queryByText, UNSAFE_getAllByProps } = renderSheet({ verdicts: [STUDIED] });

    // The arrow is drawn, not read: the link is named by its words.
    expect(getByRole('link', { name: 'Read the study' })).toBeTruthy();
    fireEvent.press(getByText('Read the study ↗'));

    await waitFor(() => expect(openURL).toHaveBeenCalledWith('https://doi.org/10.1111/sms.12104'));
    // The raw id never reaches the athlete when it resolves to a page.
    expect(queryByText('doi:10.1111/sms.12104')).toBeNull();
    // A link on its own line is still a 44pt target (DESIGN.md §8).
    const [studyLink] = UNSAFE_getAllByProps({ testID: 'verdict-study-link' });
    expect(studyLink.props.className).toContain('min-h-11');
    openURL.mockRestore();
  });

  it('names the conversation and expands the reply on a press, the claim marked', () => {
    const { getByTestId, queryByTestId, getByText } = renderSheet({ verdicts: [STUDIED], source: SOURCE });

    expect(getByTestId('verdict-source')).toHaveTextContent(/Easy Tuesday run/);
    expect(queryByTestId('verdict-source-preview')).toBeNull();

    fireEvent.press(getByTestId('verdict-source'));

    const preview = getByTestId('verdict-source-preview');
    expect(preview).toHaveTextContent(/No HR zones on file, so this is an estimate\./);
    // The claim is its own span inside the passage, without the list bullet.
    expect(getByText('If it felt smooth, count the session as a good one.').props.className).toContain(
      'bg-primary-container',
    );

    fireEvent.press(getByTestId('verdict-source'));
    expect(queryByTestId('verdict-source-preview')).toBeNull();
  });

  it('closes the sheet from the expanded reply, onto the reply the chip hangs under', () => {
    const { getByTestId, onClose } = renderSheet({ verdicts: [STUDIED], source: SOURCE });

    fireEvent.press(getByTestId('verdict-source'));
    fireEvent.press(getByTestId('verdict-see-in-conversation'));

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('draws no conversation section when the host names none', () => {
    const { queryByTestId, queryByText } = renderSheet({ verdicts: [STUDIED] });

    expect(queryByTestId('verdict-source')).toBeNull();
    expect(queryByText('Conversation')).toBeNull();
  });

  it('hands support the pressed card\'s row from the platform menu', () => {
    const onCopyReference = jest.fn();
    const { getAllByTestId } = renderSheet({ verdicts: [FIRST, STUDIED], onCopyReference });

    fireEvent.press(getAllByTestId('verdict-actions')[1]);

    expect(mockPresentMenu).toHaveBeenCalledTimes(1);
    const [rows, options] = mockPresentMenu.mock.calls[0] as [
      { label: string; onPress: () => void }[],
      { title: string; cancelLabel: string },
    ];
    expect(rows.map((r) => r.label)).toEqual(['Copy reference for support']);
    expect(options.title).toBe('Verdict actions');
    rows[0].onPress();
    expect(onCopyReference).toHaveBeenCalledWith(STUDIED);
  });

  it('offers no actions menu when the host cannot copy', () => {
    const { queryByTestId } = renderSheet({ verdicts: [STUDIED] });

    expect(queryByTestId('verdict-actions')).toBeNull();
  });

  // Boreal v2.2 P3.6: a verdict is a section, not a stack of pills — no
  // rounded-full anywhere inside it, and the ask action is an inline link.
  // Its studies and its conversation are inline links too.
  it('draws no pill inside a verdict and no filled button under it', () => {
    const { getByTestId, UNSAFE_getAllByProps } = renderSheet({
      verdicts: [FIRST],
      source: SOURCE,
      onCopyReference: jest.fn(),
    });

    fireEvent.press(getByTestId('verdict-source'));
    const card = getByTestId('verdict-card');
    const inside = classNames(card.children);
    expect(inside.length).toBeGreaterThan(0);
    expect(inside.some((c) => /\brounded-full\b/.test(c))).toBe(false);
    expect(within(card).getByText('contradicted').props.className).toContain('font-semibold');

    const [ask] = UNSAFE_getAllByProps({ testID: 'verdict-ask' });
    expect(ask.props.className).toContain('text-primary');
    expect(ask.props.className).not.toContain('bg-primary');
    expect(getByTestId('verdict-sheet').props.className).toContain('rounded-t-3xl');
  });
});
