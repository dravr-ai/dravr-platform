// ABOUTME: Unit tests for the one verdict severity rollup web and mobile share
// ABOUTME: Red if the two clients ever disagree about which flagged claim is the worst one

import { describe, it, expect } from 'vitest';
import type { ClaimVerdict } from '@pierre/shared-types';
import {
  mergeVerdictSeverities,
  summarizeVerdicts,
  verdictChipSeverity,
  verdictSummaryLabel,
} from '@pierre/shared-types';

function row(overrides: Partial<ClaimVerdict> = {}): ClaimVerdict {
  return {
    id: 'v1',
    conversation_id: 'conv-1',
    message_id: 'msg-1',
    coach_id: 'coach-1',
    claim_text: 'Creatine at 5g per day improves high-intensity performance.',
    category: 'supplement',
    status: 'supported',
    evidence_strength: 'strong',
    confidence: 0.8,
    layer_fired: 'evidence',
    explanation: null,
    evidence_refs: null,
    created_at: '2026-08-24T10:00:00Z',
    ...overrides,
  };
}

describe('summarizeVerdicts', () => {
  it('draws no chip for an empty set', () => {
    expect(summarizeVerdicts([])).toBeNull();
  });

  it('picks the worst status and the weakest evidence across the set', () => {
    const summary = summarizeVerdicts([
      { status: 'supported', evidence_strength: 'strong' },
      { status: 'unsupported', evidence_strength: 'mixed' },
      { status: 'contradicted', evidence_strength: 'weak' },
    ]);

    expect(summary).toEqual({
      worstStatus: 'contradicted',
      worstStrength: 'weak',
      count: 3,
      tone: 'error',
    });
  });

  it('reports no strength when the set carried none', () => {
    const summary = summarizeVerdicts([{ status: 'unsupported' }]);
    expect(summary?.worstStrength).toBeNull();
    expect(summary?.tone).toBe('warning');
  });
});

describe('verdictChipSeverity', () => {
  it('reads the block chip\'s single flag as the server\'s own two-state verdict', () => {
    expect(verdictChipSeverity({ claim: 'x', contradicted: true })).toEqual({
      status: 'contradicted',
    });
    expect(verdictChipSeverity({ claim: 'x', contradicted: false })).toEqual({
      status: 'unsupported',
    });
  });
});

describe('mergeVerdictSeverities', () => {
  it('counts a claim carried by both the rows and the chips exactly once', () => {
    const merged = mergeVerdictSeverities(
      [row({ claim_text: 'Your VO2max is 82.', status: 'contradicted', evidence_strength: 'none' })],
      [{ claim: 'Your VO2max is 82.', contradicted: true }],
    );

    expect(merged).toEqual([{ status: 'contradicted', evidence_strength: 'none' }]);
  });

  it('keeps a chip whose claim no row covers', () => {
    const merged = mergeVerdictSeverities(
      [row({ claim_text: 'A', status: 'supported', evidence_strength: 'strong' })],
      [{ claim: 'B', contradicted: false }],
    );

    expect(merged).toEqual([
      { status: 'supported', evidence_strength: 'strong' },
      { status: 'unsupported' },
    ]);
  });
});

describe('verdictSummaryLabel', () => {
  it('qualifies with the evidence strength where the surface read the rows', () => {
    const summary = summarizeVerdicts([{ status: 'contradicted', evidence_strength: 'none' }]);
    expect(summary).not.toBeNull();
    expect(verdictSummaryLabel(summary!)).toBe('1 verdict · none');
  });

  it('qualifies with the status where only the turn\'s chips arrived', () => {
    const summary = summarizeVerdicts([
      verdictChipSeverity({ claim: 'a', contradicted: true }),
      verdictChipSeverity({ claim: 'b', contradicted: false }),
    ]);
    expect(summary).not.toBeNull();
    expect(verdictSummaryLabel(summary!)).toBe('2 verdicts · contradicted');
  });
});
