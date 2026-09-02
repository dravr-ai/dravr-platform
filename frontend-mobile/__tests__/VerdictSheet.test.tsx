// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the verdict sheet a reply's chip opens — one card per row, its words, its loading line
// ABOUTME: The ask action hands back the exact row pressed, so the composer quotes the right claim

import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';
import type { ClaimVerdict } from '@pierre/shared-types';
import { VerdictSheet, type VerdictSheetProps } from '../src/screens/chat/VerdictSheet';

function row(overrides: Partial<ClaimVerdict> & { id: string }): ClaimVerdict {
  return {
    conversation_id: 'conv-1',
    message_id: 'msg-1',
    coach_id: 'coach-tempo',
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
    expect(getByText('evidence: none')).toBeTruthy();
    expect(getByText('confidence: 91%')).toBeTruthy();
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
});
