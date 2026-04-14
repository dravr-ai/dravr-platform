// ABOUTME: Sprint C4 tests — MessageItem renders Tier 5.5 verdict chips on assistant messages
// ABOUTME: Asserts chip visibility, drawer-open callback, and ask-about-claim shortcut
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import MessageItem from '../MessageItem';
import type { Message } from '../types';
import type { ChatVerdictRow } from '@pierre/api-client';

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

function verdictForMessage(messageId: string, overrides: Partial<ChatVerdictRow> = {}): ChatVerdictRow {
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

describe('MessageItem Tier 5.5 verdict chip', () => {
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
    expect(screen.getByText(/1 verdict/)).toBeInTheDocument();
    expect(screen.getByText(/strong/)).toBeInTheDocument();
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
    const verdicts: ChatVerdictRow[] = [
      verdictForMessage('msg-1', { id: 'v1', status: 'supported', evidence_strength: 'strong' }),
      verdictForMessage('msg-1', { id: 'v2', status: 'unsupported', evidence_strength: 'mixed' }),
      verdictForMessage('msg-1', { id: 'v3', status: 'contradicted', evidence_strength: 'weak' }),
    ];
    render(<MessageItem message={assistantMessage()} verdicts={verdicts} />);
    expect(screen.getByText(/3 verdicts/)).toBeInTheDocument();
    // The worst evidence strength among (strong, mixed, weak) is "weak".
    expect(screen.getByText(/weak/)).toBeInTheDocument();
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
    fireEvent.click(screen.getByText(/1 verdict/));
    expect(onShowVerdict).toHaveBeenCalledWith(verdict);
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
