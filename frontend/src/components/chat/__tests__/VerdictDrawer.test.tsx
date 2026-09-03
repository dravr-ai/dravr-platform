// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the verdict drawer's subline — the count alone, in the reader's language
// ABOUTME: It used to borrow the chip's string and scrub the leftover separator back off with a regex

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import VerdictDrawer from '../VerdictDrawer';
import type { ClaimVerdict } from '@pierre/shared-types';

function verdict(id: string, overrides: Partial<ClaimVerdict> = {}): ClaimVerdict {
  return {
    id,
    conversation_id: 'conv-1',
    message_id: 'msg-1',
    coach_id: 'coach-1',
    claim_text: 'Creatine at 5 g per day improves high-intensity performance.',
    category: 'supplement',
    status: 'supported',
    evidence_strength: 'strong',
    confidence: 0.8,
    layer_fired: 'evidence',
    explanation: null,
    evidence_refs: null,
    created_at: '2026-04-13T18:00:01Z',
    ...overrides,
  };
}

describe('VerdictDrawer subline', () => {
  it('says the count alone when a reply drew several verdicts', () => {
    render(
      <VerdictDrawer
        verdicts={[verdict('v1'), verdict('v2', { status: 'contradicted' })]}
        onClose={vi.fn()}
      />,
    );

    // The chip's string is "{{count}} verdicts · {{qualifier}}", and the
    // drawer used to pass an empty qualifier and strip the dangling separator
    // with a regex — which only ever knew the separator English and French
    // happen to use. The subline has its own key now.
    const subline = screen.getByText(/2 verdicts/);
    expect(subline.textContent).toBe('2 verdicts');
    expect(subline.textContent).not.toMatch(/·/);
  });

  it('keeps the status qualifier when there is exactly one', () => {
    render(<VerdictDrawer verdicts={[verdict('v1')]} onClose={vi.fn()} />);

    expect(screen.getByText('1 verdict · supported')).toBeInTheDocument();
  });

  it('says it is still reading while the rows are in flight', () => {
    render(<VerdictDrawer verdicts={[]} loading onClose={vi.fn()} />);

    // The subline and the empty body both say it — the header while the count
    // is unknown, the body where the cards will land.
    expect(screen.getAllByText('Loading verdicts…')).toHaveLength(2);
  });
});
