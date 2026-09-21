// ABOUTME: Tests for the admin triage panel under a verdict card — the knob pointer and the disposition write
// ABOUTME: The control hands the chosen call, reason and trimmed note to the writer and surfaces its failure
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import VerdictTriagePanel from '../VerdictTriagePanel';
import type { ClaimVerdict } from '@pierre/shared-types';
import type { VerdictKnob } from '../../services/api/admin';

function verdict(id: string, overrides: Partial<ClaimVerdict> = {}): ClaimVerdict {
  return {
    id,
    conversation_id: 'conv-1',
    message_id: 'msg-1',
    agent_id: 'coach-1',
    claim_text: 'Your max heart rate is 250 bpm.',
    category: 'physiological',
    status: 'contradicted',
    evidence_strength: 'strong',
    confidence: 0.95,
    layer_fired: 'deterministic',
    explanation: null,
    evidence_refs: null,
    created_at: '2026-04-13T18:00:01Z',
    ...overrides,
  };
}

const knob: VerdictKnob = {
  layer: 'deterministic',
  kind: 'deterministic_bounds',
  location: 'crates/pierre-evals/src/deterministic_bounds.rs',
  detail: '`check_physiological` holds the population bounds for `physiological`.',
  propositions: [],
};

describe('VerdictTriagePanel', () => {
  it('renders the knob the detail read resolved, and says so while it has not', () => {
    const onSetDisposition = vi.fn().mockResolvedValue(undefined);
    const { rerender } = render(
      <VerdictTriagePanel verdict={verdict('v1')} knob={undefined} onSetDisposition={onSetDisposition} />,
    );
    expect(screen.getByTestId('verdict-knob')).toHaveTextContent('Resolving the knob…');

    rerender(
      <VerdictTriagePanel verdict={verdict('v1')} knob={knob} onSetDisposition={onSetDisposition} />,
    );
    expect(screen.getByText('Deterministic bounds')).toBeInTheDocument();
    expect(screen.getByText('crates/pierre-evals/src/deterministic_bounds.rs')).toBeInTheDocument();
    expect(screen.getByText(/holds the population bounds/)).toBeInTheDocument();
  });

  it('lists the evidence propositions with the cited one marked', () => {
    render(
      <VerdictTriagePanel
        verdict={verdict('v1', { layer_fired: 'evidence', category: 'supplement', status: 'supported' })}
        knob={{
          layer: 'evidence',
          kind: 'evidence_corpus',
          location: 'evidence/sports_science/supplement/',
          detail: '2 proposition(s) keyword-match the claim.',
          propositions: [
            {
              id: 'doi:10.1186/s12970-017-0173-z',
              category: 'supplement',
              slug: 'kreider-2017-creatine',
              path: 'evidence/sports_science/supplement/kreider-2017-creatine.md',
              strength: 'strong',
              score: 4,
              cited: true,
            },
            {
              id: 'doi:10.1186/s12970-020-00383-4',
              category: 'supplement',
              slug: 'guest-2021-caffeine',
              path: 'evidence/sports_science/supplement/guest-2021-caffeine.md',
              strength: 'strong',
              score: 2,
              cited: false,
            },
          ],
        }}
        onSetDisposition={vi.fn()}
      />,
    );

    expect(screen.getByText('Evidence corpus')).toBeInTheDocument();
    expect(
      screen.getByText('evidence/sports_science/supplement/kreider-2017-creatine.md'),
    ).toBeInTheDocument();
    expect(
      screen.getByText('evidence/sports_science/supplement/guest-2021-caffeine.md'),
    ).toBeInTheDocument();
    expect(screen.getAllByText('cited')).toHaveLength(1);
    expect(screen.getByText('score 4')).toBeInTheDocument();
  });

  it('hands the chosen call, reason and trimmed note to onSetDisposition', async () => {
    const onSetDisposition = vi.fn().mockResolvedValue(undefined);
    const row = verdict('v1');
    render(<VerdictTriagePanel verdict={row} knob={knob} onSetDisposition={onSetDisposition} />);

    // Nothing chosen yet: there is nothing to write.
    expect(screen.getByRole('button', { name: 'Save disposition' })).toBeDisabled();

    fireEvent.change(screen.getByLabelText('Disposition call'), { target: { value: 'true_catch' } });
    fireEvent.change(screen.getByLabelText('Disposition reason'), { target: { value: 'other' } });
    fireEvent.change(screen.getByLabelText('Disposition note'), {
      target: { value: ' 250 bpm is nonsense ' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save disposition' }));

    await waitFor(() => {
      expect(onSetDisposition).toHaveBeenCalledWith(row, {
        disposition: 'true_catch',
        reason: 'other',
        note: '250 bpm is nonsense',
      });
    });
  });

  it('starts from the disposition already on the row and shows who wrote it', () => {
    render(
      <VerdictTriagePanel
        verdict={verdict('v1', {
          disposition: 'false_positive',
          disposition_reason: 'missing_keyword',
          disposition_note: 'wording',
          disposed_by: 'support@example.com',
          disposed_at: '2026-09-21T12:00:00Z',
        })}
        knob={knob}
        onSetDisposition={vi.fn()}
      />,
    );

    expect(screen.getByLabelText('Disposition call')).toHaveValue('false_positive');
    expect(screen.getByLabelText('Disposition reason')).toHaveValue('missing_keyword');
    expect(screen.getByLabelText('Disposition note')).toHaveValue('wording');
    expect(screen.getByText(/Disposed by support@example.com/)).toBeInTheDocument();
  });

  it('surfaces the write failure instead of swallowing it', async () => {
    const onSetDisposition = vi
      .fn()
      .mockRejectedValue(new Error('Permission required: manage_configuration'));
    render(<VerdictTriagePanel verdict={verdict('v1')} knob={knob} onSetDisposition={onSetDisposition} />);

    fireEvent.change(screen.getByLabelText('Disposition call'), { target: { value: 'unsure' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save disposition' }));

    await waitFor(() => {
      expect(screen.getByText('Permission required: manage_configuration')).toBeInTheDocument();
    });
  });
});
