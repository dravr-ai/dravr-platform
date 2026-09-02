// ABOUTME: Sprint C4 tests — MessageItem renders claim verdict chips on assistant messages
// ABOUTME: Asserts chip visibility, drawer-open callback, and ask-about-claim shortcut
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import MessageItem from '../MessageItem';
import type { Message } from '../types';
import type { ClaimVerdict } from '@pierre/shared-types';

function assistantMessage(): Message {
  return {
    id: 'msg-1',
    role: 'assistant',
    content: 'Take 5 g of creatine per day for high-intensity performance.',
    created_at: '2026-04-13T18:00:00Z',
  };
}

function userMessage(): Message {
  return {
    id: 'msg-2',
    role: 'user',
    content: 'How much creatine should I take?',
    created_at: '2026-04-13T17:59:00Z',
  };
}

function verdictForMessage(messageId: string, overrides: Partial<ClaimVerdict> = {}): ClaimVerdict {
  return {
    id: 'v1',
    conversation_id: 'conv-1',
    message_id: messageId,
    coach_id: 'coach-1',
    claim_text: 'Creatine at 5g per day improves high-intensity performance.',
    category: 'supplement',
    status: 'supported',
    evidence_strength: 'strong',
    confidence: 0.8,
    layer_fired: 'evidence',
    explanation: 'Backed by ISSN 2017 position stand on creatine.',
    evidence_refs: 'issn:2017-creatine',
    created_at: '2026-04-13T18:00:01Z',
    ...overrides,
  };
}

describe('MessageItem claim verdict chip', () => {
  it('does not render a chip when no verdicts exist', () => {
    render(<MessageItem message={assistantMessage()} verdicts={[]} />);
    expect(screen.queryByText(/verdict/)).not.toBeInTheDocument();
  });

  it('renders a chip on an assistant message with one verdict', () => {
    render(
      <MessageItem
        message={assistantMessage()}
        verdicts={[verdictForMessage('msg-1')]}
      />,
    );
    expect(screen.getByText('1 verdict · supported')).toBeInTheDocument();
    // The evidence strength is a per-claim detail the drawer prints; the chip
    // qualifies with the status word alone.
    expect(screen.queryByText(/strong/)).not.toBeInTheDocument();
  });

  it('does not render a chip on user messages', () => {
    render(
      <MessageItem
        message={userMessage()}
        verdicts={[verdictForMessage('msg-2')]}
      />,
    );
    expect(screen.queryByText(/verdict/)).not.toBeInTheDocument();
  });

  it('summarizes worst-status across multiple verdicts', () => {
    const verdicts: ClaimVerdict[] = [
      verdictForMessage('msg-1', { id: 'v1', status: 'supported', evidence_strength: 'strong' }),
      verdictForMessage('msg-1', { id: 'v2', status: 'unsupported', evidence_strength: 'mixed' }),
      verdictForMessage('msg-1', { id: 'v3', status: 'contradicted', evidence_strength: 'weak' }),
    ];
    render(<MessageItem message={assistantMessage()} verdicts={verdicts} />);
    // The worst status among (supported, unsupported, contradicted) is
    // "contradicted"; the chip never mentions an evidence strength.
    expect(screen.getByText('3 verdicts · contradicted')).toBeInTheDocument();
    expect(screen.queryByText(/weak/)).not.toBeInTheDocument();
  });

  it('counts the rows alone once they exist, even when the chips spell the claim differently', () => {
    const verdict = verdictForMessage('msg-1', {
      claim_text: 'Creatine at 5g per day improves high-intensity performance.',
      status: 'contradicted',
      evidence_strength: 'none',
    });
    render(
      <MessageItem
        message={assistantMessage()}
        blocks={[
          { type: 'prose', text: 'Take 5 g of creatine per day for high-intensity performance.' },
          {
            type: 'verdicts',
            chips: [
              { claim: 'Take 5 g of creatine per day for high-intensity performance.', contradicted: true },
              { claim: 'Six hours of sleep is enough.', contradicted: false },
            ],
          },
        ]}
        verdicts={[verdict]}
      />,
    );
    expect(screen.getByText('1 verdict · contradicted')).toBeInTheDocument();
    expect(screen.queryByText(/3 verdicts/)).not.toBeInTheDocument();
  });

  it('draws the plain shield for a reply whose every claim was supported', () => {
    render(
      <MessageItem
        message={assistantMessage()}
        verdicts={[verdictForMessage('msg-1', { status: 'supported', evidence_strength: 'strong' })]}
      />,
    );
    const chip = screen.getByTestId('verdict-chip');
    expect(chip.querySelector('svg.lucide-shield')).not.toBeNull();
    expect(chip.querySelector('svg.lucide-shield-alert')).toBeNull();
  });

  it('draws the alert shield for a reply with a contradicted claim, and speaks no raw enum in its tooltip', () => {
    render(
      <MessageItem
        message={assistantMessage()}
        verdicts={[
          verdictForMessage('msg-1', { id: 'v1', status: 'supported', evidence_strength: 'strong' }),
          verdictForMessage('msg-1', { id: 'v2', status: 'contradicted', evidence_strength: 'none' }),
        ]}
      />,
    );
    const chip = screen.getByTestId('verdict-chip');
    expect(chip.querySelector('svg.lucide-shield-alert')).not.toBeNull();
    // The tooltip's words come from the corpus — 'contradicted' and 'none'
    // happen to be the English words too, so pin the shape instead: the
    // status and strength are rendered as words, not left as enum tokens.
    expect(chip).toHaveAttribute('title', '2 claim verdicts — worst: contradicted, none');
  });

  it('opens the drawer with no rows when only the turn\'s chips have landed', () => {
    const onShowVerdict = vi.fn();
    render(
      <MessageItem
        message={assistantMessage()}
        blocks={[
          { type: 'prose', text: 'Take 5 g of creatine per day for high-intensity performance.' },
          {
            type: 'verdicts',
            chips: [{ claim: 'Take 5 g of creatine per day for high-intensity performance.', contradicted: true }],
          },
        ]}
        verdicts={[]}
        onShowVerdict={onShowVerdict}
      />,
    );
    expect(screen.getByText('1 verdict · contradicted')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('verdict-chip'));
    expect(onShowVerdict).toHaveBeenCalledWith([], 'msg-1');
  });

  it('calls onShowVerdict when the chip is clicked', () => {
    const onShowVerdict = vi.fn();
    const verdict = verdictForMessage('msg-1');
    render(
      <MessageItem
        message={assistantMessage()}
        verdicts={[verdict]}
        onShowVerdict={onShowVerdict}
      />,
    );
    fireEvent.click(screen.getByTestId('verdict-chip'));
    expect(onShowVerdict).toHaveBeenCalledWith([verdict], 'msg-1');
  });

  it('renders the Ask me about this claim shortcut for single-verdict messages', () => {
    const onAskAboutClaim = vi.fn();
    render(
      <MessageItem
        message={assistantMessage()}
        verdicts={[verdictForMessage('msg-1')]}
        onAskAboutClaim={onAskAboutClaim}
      />,
    );
    const button = screen.getByRole('button', { name: /Ask me about this claim/i });
    fireEvent.click(button);
    expect(onAskAboutClaim).toHaveBeenCalled();
  });
});
