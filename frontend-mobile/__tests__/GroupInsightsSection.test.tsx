// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that a group's health flags and weekly report read in the admin's language on the phone
// ABOUTME: The server sends evidence and stats only, so a French admin must see French lines and a decimal comma

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import type { GroupHealthFlagsResponse, GroupWeeklyReportResponse } from '@pierre/shared-types';

const mockReport: GroupWeeklyReportResponse = {
  report: {
    stats: {
      total_members: 3,
      active_members: 2,
      avg_weekly_volume_km: 140.83,
      avg_ctl: 100,
      flagged_members: 0,
      weekly_trend: 'declining',
    },
    fresh_members: [{ user_id: 'user-phil', display_name: 'Phil', form_pct: 12, tsb: 12 }],
  },
};

const mockHealth: GroupHealthFlagsResponse = {
  flags: [
    {
      user_id: 'user-marie',
      display_name: 'Marie',
      flag_type: 'deep_fatigue',
      severity: 'critical',
      evidence: { kind: 'form_share', form_pct: -40, tsb: -40 },
    },
    {
      user_id: 'user-luc',
      display_name: 'Luc',
      flag_type: 'inactive',
      severity: 'warning',
      evidence: { kind: 'inactive_days', days: 10 },
    },
  ],
  total: 2,
};

jest.mock('../src/services/api', () => ({
  groupsApi: {
    getWeeklyReport: jest.fn(() => Promise.resolve(mockReport)),
    getHealthFlags: jest.fn(() => Promise.resolve(mockHealth)),
  },
}));

import { GroupInsightsSection } from '../src/screens/groups/GroupInsightsSection';

function renderSection() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  return render(
    <QueryClientProvider client={client}>
      <GroupInsightsSection groupId="group-1" isAdmin weeklyDigestEnabled />
    </QueryClientProvider>,
  );
}

describe('GroupInsightsSection', () => {
  afterEach(async () => {
    await i18n.changeLanguage('en');
  });

  it('phrases a French admin’s flags, report and decimals in French', async () => {
    await i18n.changeLanguage('fr');
    const { getByTestId, getByText, getAllByTestId } = renderSection();

    await waitFor(() => {
      expect(getByTestId('group-report-summary').props.children).toBe(
        '2/3 membres actifs cette semaine, 140,8 km en moyenne par membre.',
      );
    });
    expect(getByText('Forme à -40 % de sa charge chronique (TSB -40), fatigue profonde')).toBeTruthy();
    expect(getByText('Aucune activité depuis 10 jours')).toBeTruthy();
    expect(getByText('Marie : Forme à -40 % de sa charge chronique (TSB -40), fatigue profonde')).toBeTruthy();
    expect(getByText('Phil : forme fraîche (+12 % de sa charge chronique, TSB +12)')).toBeTruthy();
    expect(getAllByTestId('group-report-recommendation')).toHaveLength(1);
    expect(
      getByText(
        'Volume du groupe en baisse par rapport à la semaine dernière — prends des nouvelles des membres moins actifs.',
      ),
    ).toBeTruthy();
  });

  it('reads the same numbers in English for an English admin', async () => {
    const { getByTestId, getByText } = renderSection();

    await waitFor(() => {
      expect(getByTestId('group-report-summary').props.children).toBe(
        '2/3 members active this week, averaging 140.8 km each.',
      );
    });
    expect(getByText('Form at -40% of chronic load (TSB -40), deepest fatigue band')).toBeTruthy();
    expect(getByText('Luc: No activity for 10 days')).toBeTruthy();
  });
});
