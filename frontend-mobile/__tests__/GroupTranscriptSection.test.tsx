// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that a room entry's date and agent tag read in the app language on the phone, not the device's
// ABOUTME: A French athlete on an English-configured phone saw "1/15/2026" under French chrome

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import type { GroupTranscriptResponse } from '@pierre/shared-types';

const mockTranscript: GroupTranscriptResponse = {
  group_id: 'group-1',
  members: [],
  entries: [
    {
      id: 'entry-1',
      speaker: 'member',
      withheld: false,
      own: false,
      author_user_id: 'user-marie',
      author_display_name: 'Marie',
      content: 'Sortie longue demain ?',
      message_id: null,
      created_at: '2026-01-15T12:00:00Z',
    },
    {
      id: 'entry-2',
      speaker: 'coach',
      withheld: false,
      own: false,
      author_user_id: 'coach-tempo',
      author_display_name: 'Tempo',
      content: 'Oui, allure facile.',
      message_id: null,
      created_at: '2026-01-15T12:01:00Z',
    },
  ],
};

jest.mock('../src/services/api', () => ({
  groupsApi: {
    getTranscript: jest.fn(() => Promise.resolve(mockTranscript)),
  },
}));

import { GroupTranscriptSection } from '../src/screens/groups/GroupTranscriptSection';

function renderSection() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  return render(
    <QueryClientProvider client={client}>
      <GroupTranscriptSection groupId="group-1" />
    </QueryClientProvider>,
  );
}

describe('GroupTranscriptSection', () => {
  afterEach(async () => {
    await i18n.changeLanguage('en');
  });

  it('dates a room entry in French for a French athlete', async () => {
    await i18n.changeLanguage('fr');
    const { getAllByText, getByText, queryByText } = renderSection();

    await waitFor(() => {
      expect(getByText('Sortie longue demain ?')).toBeTruthy();
    });
    expect(getAllByText('15 janv. 2026')).toHaveLength(2);
    expect(queryByText('1/15/2026')).toBeNull();
    // The agent's tag is the catalogue's word, not an English suffix.
    expect(getByText('Tempo · Agent')).toBeTruthy();
    expect(queryByText('Tempo · agent')).toBeNull();
  });

  it('dates the same entry in English for an English athlete', async () => {
    const { getAllByText } = renderSection();

    await waitFor(() => {
      expect(getAllByText('Jan 15, 2026')).toHaveLength(2);
    });
  });
});
