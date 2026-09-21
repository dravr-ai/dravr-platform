// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the verdict drawer's subline — the count alone, in the reader's language
// ABOUTME: And its triage slot: the admin's panel renders under each card, the chat surface gets nothing

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import VerdictDrawer from '../VerdictDrawer';
import type { ClaimVerdict } from '@pierre/shared-types';

function verdict(id: string, overrides: Partial<ClaimVerdict> = {}): ClaimVerdict {
  return {
    id,
    conversation_id: 'conv-1',
    message_id: 'msg-1',
    agent_id: 'coach-1',
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


describe('VerdictDrawer triage slot', () => {
  it('renders nothing extra on the chat surface', () => {
    render(<VerdictDrawer verdicts={[verdict('v1')]} onClose={vi.fn()} />);

    expect(screen.queryByTestId('triage-slot')).not.toBeInTheDocument();
  });

  it('renders what the admin passes under each card, once per verdict', () => {
    const renderTriage = vi.fn((v: ClaimVerdict) => (
      <div data-testid="triage-slot">triage for {v.id}</div>
    ));
    render(
      <VerdictDrawer
        verdicts={[verdict('v1'), verdict('v2', { status: 'contradicted' })]}
        onClose={vi.fn()}
        renderTriage={renderTriage}
      />,
    );

    const slots = screen.getAllByTestId('triage-slot');
    expect(slots).toHaveLength(2);
    expect(slots[0]).toHaveTextContent('triage for v1');
    expect(slots[1]).toHaveTextContent('triage for v2');
    // The slot is a function of the verdict, so the panel it renders can
    // read the row's own disposition and knob.
    expect(renderTriage).toHaveBeenCalledWith(expect.objectContaining({ id: 'v1' }));
    expect(renderTriage).toHaveBeenCalledWith(expect.objectContaining({ id: 'v2' }));
  });
});
