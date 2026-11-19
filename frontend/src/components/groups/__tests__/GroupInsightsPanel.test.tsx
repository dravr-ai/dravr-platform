// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins a health flag's severity pill to the badge tone table
// ABOUTME: Critical draws as an error, warning as a warning, info in the muted tone, so every ink clears AA

import React from 'react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { GroupHealthFlag } from '@pierre/shared-types';

const FLAGS: GroupHealthFlag[] = [
  { user_id: 'u1', display_name: 'Ana', flag_type: 'overreaching', severity: 'critical', evidence: { kind: 'overtraining_risk' } },
  { user_id: 'u2', display_name: 'Ben', flag_type: 'deep_fatigue', severity: 'warning', evidence: { kind: 'overtraining_risk' } },
  { user_id: 'u3', display_name: 'Cy', flag_type: 'inactive', severity: 'info', evidence: { kind: 'inactive_days', days: 9 } },
];

// The data seam: the two hooks are React Query reads of the groups API. The
// panel renders their answers, so the fixture stands in for a loaded report.
vi.mock('../../../hooks/useGroups', () => ({
  useGroupWeeklyReport: () => ({ report: null, isLoading: false }),
  useGroupHealthFlags: () => ({ flags: FLAGS, isLoading: false }),
}));

import GroupInsightsPanel from '../GroupInsightsPanel';

describe('GroupInsightsPanel', () => {
  it('draws each health flag in its severity tone', () => {
    render(<GroupInsightsPanel groupId="g1" isAdmin weeklyDigestEnabled />);

    expect(screen.getByText('Overreaching')).toHaveClass('badge-error');
    expect(screen.getByText('Deep fatigue')).toHaveClass('badge-warning');
    expect(screen.getByText('Inactive')).toHaveClass('badge-secondary');
  });
});
