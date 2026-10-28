// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the one verdict rollup web and mobile share — headline status, per-status counts, tone
// ABOUTME: Red if an abstention labels a reply whose claims held, or a finding stops leading the chip (carnet#1189)

import { describe, it, expect } from 'vitest';
import {
  VERDICT_STATUS_ORDER,
  summarizeVerdicts,
  verdictChipSeverity,
  verdictToneAlerts,
} from '../src/verdict';
import type { ClaimVerdictStatus, VerdictSeverity } from '../src/verdict';

function of(...statuses: ClaimVerdictStatus[]): VerdictSeverity[] {
  return statuses.map((status) => ({ status }));
}

describe('summarizeVerdicts', () => {
  it('draws no chip for an empty set', () => {
    expect(summarizeVerdicts([])).toBeNull();
  });

  it('leads with a contradicted claim and keeps the weakest evidence behind it', () => {
    const summary = summarizeVerdicts([
      { status: 'supported', evidence_strength: 'strong' },
      { status: 'unsupported', evidence_strength: 'mixed' },
      { status: 'contradicted', evidence_strength: 'weak' },
    ]);

    expect(summary).toEqual({
      headlineStatus: 'contradicted',
      statusCounts: { contradicted: 1, unsupported: 1, supported: 1, unverifiable: 0, rhetorical: 0 },
      worstStrength: 'weak',
      count: 3,
      tone: 'error',
    });
  });

  it('never lets an abstention outrank a supported claim', () => {
    // Three claims checked against the record and held, one forward-looking
    // prescription the verifier could not check: the reply reads as supported.
    const summary = summarizeVerdicts([
      { status: 'supported', evidence_strength: 'strong' },
      { status: 'supported', evidence_strength: 'mixed' },
      { status: 'supported', evidence_strength: 'strong' },
      { status: 'unverifiable', evidence_strength: 'none' },
    ]);

    expect(summary?.headlineStatus).toBe('supported');
    expect(summary?.tone).toBe('success');
    expect(summary?.statusCounts.supported).toBe(3);
    expect(summary?.statusCounts.unverifiable).toBe(1);
    // The abstention's « none » does not qualify the claims that held.
    expect(summary?.worstStrength).toBe('mixed');
    expect(verdictToneAlerts(summary?.tone ?? 'error')).toBe(false);
  });

  it('keeps a rhetorical claim behind a supported one too', () => {
    expect(summarizeVerdicts(of('rhetorical', 'supported'))?.headlineStatus).toBe('supported');
  });

  it('still lets an unsupported claim win over supported and abstaining ones', () => {
    const summary = summarizeVerdicts(of('supported', 'unverifiable', 'unsupported', 'rhetorical'));
    expect(summary?.headlineStatus).toBe('unsupported');
    expect(summary?.tone).toBe('warning');
  });

  it('still lets a contradicted claim win over an unsupported one', () => {
    expect(summarizeVerdicts(of('unsupported', 'contradicted'))?.headlineStatus).toBe('contradicted');
  });

  it('headlines an all-abstention reply by its abstention', () => {
    expect(summarizeVerdicts(of('unverifiable'))?.headlineStatus).toBe('unverifiable');
    expect(summarizeVerdicts(of('rhetorical', 'unverifiable'))?.headlineStatus).toBe('unverifiable');
    expect(summarizeVerdicts(of('rhetorical'))?.headlineStatus).toBe('rhetorical');
    expect(summarizeVerdicts(of('rhetorical'))?.tone).toBe('info');
  });

  it('reports no strength when the headline verdicts carried none', () => {
    const summary = summarizeVerdicts([{ status: 'unsupported' }]);
    expect(summary?.worstStrength).toBeNull();
    expect(summary?.tone).toBe('warning');
  });

  it('counts every status, and the counts add up to the set', () => {
    const summary = summarizeVerdicts(of('supported', 'supported', 'unverifiable', 'rhetorical', 'contradicted'));
    expect(summary?.statusCounts).toEqual({
      contradicted: 1,
      unsupported: 0,
      supported: 2,
      unverifiable: 1,
      rhetorical: 1,
    });
    const total = Object.values(summary?.statusCounts ?? {}).reduce((a, b) => a + b, 0);
    expect(total).toBe(summary?.count);
  });
});

describe('VERDICT_STATUS_ORDER', () => {
  it('names findings first, then supported, then the abstentions', () => {
    expect(VERDICT_STATUS_ORDER).toEqual(['contradicted', 'unsupported', 'supported', 'unverifiable', 'rhetorical']);
  });

  it('is the order the rollup picks its headline in', () => {
    // Any two statuses: the one listed first is the headline of the pair.
    for (const [i, first] of VERDICT_STATUS_ORDER.entries()) {
      for (const later of VERDICT_STATUS_ORDER.slice(i + 1)) {
        expect(summarizeVerdicts(of(later, first))?.headlineStatus).toBe(first);
      }
    }
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

describe('verdictToneAlerts', () => {
  it('alerts for a contradicted or unsupported claim and for nothing milder', () => {
    expect(verdictToneAlerts('error')).toBe(true);
    expect(verdictToneAlerts('warning')).toBe(true);
    expect(verdictToneAlerts('info')).toBe(false);
    expect(verdictToneAlerts('secondary')).toBe(false);
    expect(verdictToneAlerts('success')).toBe(false);
  });
});
