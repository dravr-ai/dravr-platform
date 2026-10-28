// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the feedback thread — the team's welcome, the athlete's messages, and what a sent message carries
// ABOUTME: Opened from a thumbs-down it sends that reply once; a failed send keeps the draft and says so (carnet#1235)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { APP_VERSION } from '@pierre/shared-constants';
import type { FeedbackAbout, FeedbackMessage, SendFeedbackRequest } from '@pierre/shared-types';
import FeedbackThread from '../FeedbackThread';

const api = vi.hoisted(() => ({
  getThread: vi.fn<() => Promise<{ messages: FeedbackMessage[] }>>(),
  sendMessage: vi.fn<(request: SendFeedbackRequest) => Promise<FeedbackMessage>>(),
}));

vi.mock('../../../services/api', () => ({
  feedbackApi: { getThread: api.getThread, sendMessage: api.sendMessage },
}));

const SENT: FeedbackMessage = {
  id: 'm-1',
  body: 'The week view skips my indoor ride.',
  created_at: '2026-10-28T15:00:00Z',
  about: null,
};
const ABOUT: FeedbackAbout = { conversation_id: 'c-1', message_id: 'r-1' };

function renderThread(about: FeedbackAbout | null = null, onClearAbout = vi.fn()) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <FeedbackThread about={about} openedFrom={() => 'home'} onClearAbout={onClearAbout} />
    </QueryClientProvider>,
  );
  return { onClearAbout };
}

beforeEach(() => {
  vi.clearAllMocks();
  api.getThread.mockResolvedValue({ messages: [] });
});

describe('FeedbackThread', () => {
  it('opens on the team’s welcome and says who reads it', async () => {
    renderThread();
    expect(screen.getByRole('heading', { name: 'Dravr team' })).toBeInTheDocument();
    expect(screen.getByText('Read by the people who build Dravr. No AI.')).toBeInTheDocument();
    expect(screen.getByText(/We read every message\./)).toBeInTheDocument();
    expect(
      screen.getByText('Each message also carries the screen you came from, the app version and your language.'),
    ).toBeInTheDocument();
    await waitFor(() => expect(api.getThread).toHaveBeenCalledTimes(1));
  });

  it('shows the messages already sent', async () => {
    api.getThread.mockResolvedValue({ messages: [SENT, { ...SENT, id: 'm-2', body: 'And the map.', about: ABOUT }] });
    renderThread();
    expect(await screen.findByText('The week view skips my indoor ride.')).toBeInTheDocument();
    expect(screen.getByText('And the map.')).toBeInTheDocument();
    expect(screen.getByText('About an agent reply')).toBeInTheDocument();
  });

  it('sends the trimmed message with the screen, app version and language, then shows it', async () => {
    const user = userEvent.setup();
    api.sendMessage.mockResolvedValue(SENT);
    renderThread();

    const composer = screen.getByRole('textbox', { name: 'Message the Dravr team...' });
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
    await user.type(composer, '  The week view skips my indoor ride.  ');
    await user.click(screen.getByRole('button', { name: 'Send' }));

    await waitFor(() => expect(api.sendMessage).toHaveBeenCalledTimes(1));
    expect(api.sendMessage.mock.calls[0][0]).toEqual({
      body: 'The week view skips my indoor ride.',
      screen: 'home',
      app_version: APP_VERSION,
      locale: 'en',
    });
    expect(await screen.findByText('The week view skips my indoor ride.')).toBeInTheDocument();
    expect(composer).toHaveValue('');
  });

  it('sends the reply it was opened about once, then lets go of it', async () => {
    const user = userEvent.setup();
    api.sendMessage.mockResolvedValue({ ...SENT, about: ABOUT });
    const { onClearAbout } = renderThread(ABOUT);

    expect(screen.getByTestId('feedback-about')).toHaveTextContent('About an agent reply');
    await user.type(screen.getByRole('textbox'), 'Tuesday is counted twice.{Enter}');

    await waitFor(() => expect(api.sendMessage).toHaveBeenCalledTimes(1));
    expect(api.sendMessage.mock.calls[0][0].about).toEqual(ABOUT);
    await waitFor(() => expect(onClearAbout).toHaveBeenCalledTimes(1));
  });

  it('lets the athlete remove the reply before sending', async () => {
    const user = userEvent.setup();
    const { onClearAbout } = renderThread(ABOUT);
    await user.click(screen.getByRole('button', { name: 'Remove the reply' }));
    expect(onClearAbout).toHaveBeenCalledTimes(1);
  });

  it('keeps the draft and says so when a send fails', async () => {
    const user = userEvent.setup();
    api.sendMessage.mockRejectedValue(new Error('offline'));
    renderThread();

    const composer = screen.getByRole('textbox');
    await user.type(composer, 'Still here?');
    await user.click(screen.getByRole('button', { name: 'Send' }));

    expect(await screen.findByRole('alert')).toHaveTextContent("Your message didn't send. Try again.");
    expect(composer).toHaveValue('Still here?');
  });
});
