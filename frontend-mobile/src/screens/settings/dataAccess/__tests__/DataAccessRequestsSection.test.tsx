// ABOUTME: Unit tests for the mobile "Ask a company for your data" section — the form's checks, a draft written, its letter, and the steps after sending it
// ABOUTME: Runs the real shared hooks over a mocked API; a refused step is said in place and never reaches the app's global error toast (carnet#1646)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { Linking } from 'react-native';
import { MutationCache, QueryClient, QueryClientProvider } from '@tanstack/react-query';
import * as Clipboard from 'expo-clipboard';
import * as Sharing from 'expo-sharing';
import type { DataAccessRequest } from '@pierre/shared-types';
import { DataAccessRequestsSection } from '../DataAccessRequestsSection';

const mockApi = {
  list: jest.fn(),
  create: jest.fn(),
  get: jest.fn(),
  edit: jest.fn(),
  letter: jest.fn(),
  markSent: jest.fn(),
  recordExtension: jest.fn(),
  markAnswered: jest.fn(),
  close: jest.fn(),
};
// A getter: the factory runs when the section is imported, before `mockApi` is initialised.
jest.mock('../../../../services/api', () => ({
  get dataAccessRequestsApi() {
    return mockApi;
  },
}));

const mockPush = jest.fn();
jest.mock('expo-router', () => ({ useRouter: () => ({ push: mockPush }) }));
jest.mock('expo-clipboard', () => ({ setStringAsync: jest.fn(() => Promise.resolve(true)) }));

const DRAFT: DataAccessRequest = {
  id: 'r1',
  reference: 'DRV-2026-AB3XZ7',
  controller: 'Acme Fitness',
  controller_contact: 'privacy@acme.example',
  athlete_identifier: 'ana@example.com',
  law: 'gdpr',
  scope: 'access_portability',
  letter_locale: 'en',
  status: 'drafted',
  derived_state: null,
  sent_on: null,
  due_on: null,
  extended_until: null,
  answered_on: null,
  created_at: '2026-11-18T15:00:00Z',
  updated_at: '2026-11-18T15:00:00Z',
};

const LETTER = {
  locale: 'en',
  subject: 'Data subject access request — reference DRV-2026-AB3XZ7',
  body: 'Date: 18 November 2026',
  text: 'Data subject access request — reference DRV-2026-AB3XZ7\n\nDate: 18 November 2026\n',
  filename: 'DRV-2026-AB3XZ7.txt',
  mailto: 'mailto:privacy@acme.example?subject=x&body=y',
};

const globalMutationError = jest.fn();

function renderSection() {
  const client = new QueryClient({
    mutationCache: new MutationCache({ onError: globalMutationError }),
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <DataAccessRequestsSection />
    </QueryClientProvider>,
  );
}

describe('DataAccessRequestsSection (mobile)', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockApi.letter.mockResolvedValue(LETTER);
  });

  it('says there are no requests yet and opens the import screen from the fallback note', async () => {
    mockApi.list.mockResolvedValue([]);
    renderSection();

    expect(await screen.findByTestId('data-access-empty')).toBeTruthy();
    fireEvent.press(screen.getByTestId('data-access-fallback-action'));
    expect(mockPush).toHaveBeenCalledWith('/(app)/import');
  });

  it('names each missing field and sends nothing', async () => {
    mockApi.list.mockResolvedValue([]);
    renderSection();

    fireEvent.press(await screen.findByTestId('data-access-new'));
    fireEvent.press(screen.getByTestId('data-access-submit'));

    expect(await screen.findByText("Enter the company's name.")).toBeTruthy();
    expect(screen.getByText('Enter the identifier you use with that company.')).toBeTruthy();
    expect(mockApi.create).not.toHaveBeenCalled();
  });

  it('writes a draft from a suggested company and shows its letter', async () => {
    mockApi.list.mockResolvedValue([]);
    mockApi.create.mockResolvedValue({ ...DRAFT, controller: 'Strava' });
    renderSection();

    fireEvent.press(await screen.findByTestId('data-access-new'));
    fireEvent.press(screen.getByTestId('data-access-company-Strava'));
    fireEvent.changeText(screen.getByTestId('data-access-contact'), 'privacy@strava.example');
    fireEvent.changeText(screen.getByTestId('data-access-identifier'), 'ana@example.com');
    fireEvent.press(screen.getByTestId('data-access-law-gdpr'));
    fireEvent.press(screen.getByTestId('data-access-language-de'));
    fireEvent.press(screen.getByTestId('data-access-submit'));

    await waitFor(() =>
      expect(mockApi.create).toHaveBeenCalledWith({
        controller: 'Strava',
        controller_contact: 'privacy@strava.example',
        athlete_identifier: 'ana@example.com',
        law: 'gdpr',
        scope: 'access_portability',
        letter_locale: 'de',
      }),
    );
    expect(await screen.findByTestId('data-access-letter')).toHaveTextContent(/reference DRV-2026-AB3XZ7/);

    fireEvent.press(screen.getByTestId('data-access-copy'));
    await waitFor(() => expect(Clipboard.setStringAsync).toHaveBeenCalledWith(LETTER.text));

    fireEvent.press(screen.getByTestId('data-access-download'));
    await waitFor(() =>
      expect(Sharing.shareAsync).toHaveBeenCalledWith(expect.stringContaining('DRV-2026-AB3XZ7.txt'), expect.anything()),
    );

    const openURL = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
    fireEvent.press(screen.getByTestId('data-access-mailto'));
    expect(openURL).toHaveBeenCalledWith(LETTER.mailto);
  });

  it('marks a draft sent today and shows the due date the server counted', async () => {
    mockApi.list.mockResolvedValue([DRAFT]);
    mockApi.markSent.mockImplementation(async (_id: string, date: string) => ({
      ...DRAFT,
      status: 'sent',
      sent_on: date,
      due_on: '2026-12-18',
    }));
    renderSection();

    fireEvent.press(await screen.findByTestId('data-access-row-r1'));
    fireEvent.press(await screen.findByTestId('data-access-mark-sent'));
    // A screen reader adjusts the day as one element; the chevrons are hidden from it.
    const stepper = screen.getByTestId('data-access-step-date');
    const shown = screen.getByTestId('data-access-step-date-value').props.children;
    fireEvent(stepper, 'accessibilityAction', { nativeEvent: { actionName: 'decrement' } });
    expect(screen.getByTestId('data-access-step-date-value').props.children).not.toBe(shown);
    // Forward again, twice: the stepper never passes today.
    const later = screen.getByTestId('data-access-step-date-later', { includeHiddenElements: true });
    fireEvent.press(later);
    fireEvent.press(later);
    expect(screen.getByTestId('data-access-step-date-value').props.children).toBe(shown);
    fireEvent.press(screen.getByTestId('data-access-step-confirm'));

    await waitFor(() => expect(mockApi.markSent).toHaveBeenCalledTimes(1));
    const [, sentOn] = mockApi.markSent.mock.calls[0];
    expect(sentOn).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    expect(await screen.findByText('Due December 18, 2026')).toBeTruthy();
    expect(screen.getByTestId('data-access-record-extension')).toBeTruthy();
  });

  it('says a refused step in place and leaves the global toast alone', async () => {
    const sent = { ...DRAFT, status: 'sent' as const, sent_on: '2026-11-01', due_on: '2026-12-01' };
    mockApi.list.mockResolvedValue([sent]);
    mockApi.markAnswered.mockRejectedValue(Object.assign(new Error('409'), { response: { status: 409 } }));
    renderSection();

    fireEvent.press(await screen.findByTestId('data-access-row-r1'));
    fireEvent.press(await screen.findByTestId('data-access-mark-answered'));
    fireEvent.press(screen.getByTestId('data-access-step-confirm'));

    expect(await screen.findByTestId('data-access-step-failure')).toHaveTextContent(
      'Your request could not be saved. Try again in a moment.',
    );
    expect(globalMutationError).not.toHaveBeenCalled();
  });

  it('names the recourse once an answer is overdue', async () => {
    const overdue = {
      ...DRAFT,
      law: 'quebec_p39_1' as const,
      status: 'sent' as const,
      derived_state: 'overdue' as const,
      sent_on: '2026-10-01',
      due_on: '2026-10-31',
    };
    mockApi.list.mockResolvedValue([overdue]);
    renderSection();

    expect(await screen.findByText('Overdue')).toBeTruthy();
    fireEvent.press(screen.getByTestId('data-access-row-r1'));
    expect(await screen.findByTestId('data-access-recourse')).toHaveTextContent(/Commission d'accès à l'information/);
    expect(screen.queryByTestId('data-access-record-extension')).toBeNull();
  });
});
