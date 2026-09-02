// ABOUTME: Unit tests for the one verdict severity rollup web and mobile share
// ABOUTME: Red if the two clients ever disagree about which flagged claim is the worst one

import { describe, it, expect } from 'vitest';
import { summarizeVerdicts, verdictChipSeverity, verdictToneAlerts } from '@pierre/shared-types';

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

describe('verdictToneAlerts', () => {
  it('alerts for a contradicted or unsupported claim and for nothing milder', () => {
    expect(verdictToneAlerts('error')).toBe(true);
    expect(verdictToneAlerts('warning')).toBe(true);
    expect(verdictToneAlerts('info')).toBe(false);
    expect(verdictToneAlerts('secondary')).toBe(false);
    expect(verdictToneAlerts('success')).toBe(false);
  });
});
