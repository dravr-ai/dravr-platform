// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the feedback thread — the team's welcome and replies, the athlete's messages, and what a sent message carries
// ABOUTME: A team reply is the team's and marked read; a screenshot is checked, previewed and sent; a failed send keeps the draft (carnet#1392)

import { describe, it, expect, vi, beforeEach, beforeAll } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { APP_VERSION } from '@pierre/shared-constants';
import type { FeedbackAbout, FeedbackMessage, SendFeedbackRequest } from '@pierre/shared-types';
import FeedbackThread from '../FeedbackThread';

const api = vi.hoisted(() => ({
  getThread: vi.fn<() => Promise<{ messages: FeedbackMessage[] }>>(),
  sendMessage: vi.fn<(request: SendFeedbackRequest) => Promise<FeedbackMessage>>(),
  markRead: vi.fn<() => Promise<void>>(),
  getImage: vi.fn<(messageId: string) => Promise<Blob>>(),
}));

vi.mock('../../../services/api', () => ({
  feedbackApi: {
    getThread: api.getThread,
    sendMessage: api.sendMessage,
    markRead: api.markRead,
    getImage: api.getImage,
  },
}));

/** A PNG by its header, as the composer would read one from disk. */
function pngFile(name = 'shot.png', size?: number): File {
  const header = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  const bytes = new Uint8Array(size ?? header.length);
  bytes.set(header);
  return new File([bytes], name, { type: 'image/png' });
}

beforeAll(() => {
  // jsdom has no object URLs; the thread makes one for each screenshot it shows.
  URL.createObjectURL = vi.fn(() => 'blob:screenshot');
  URL.revokeObjectURL = vi.fn();
});

const SENT: FeedbackMessage = {
  id: 'm-1',
  author: 'athlete',
  body: 'The week view skips my indoor ride.',
  created_at: '2026-10-28T15:00:00Z',
  about: null,
  has_image: false,
};
const ABOUT: FeedbackAbout = { conversation_id: 'c-1', message_id: 'r-1' };

function renderThread(about: FeedbackAbout | null = null, onClearAbout = vi.fn()) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const { unmount } = render(
    <QueryClientProvider client={client}>
      <FeedbackThread about={about} openedFrom={() => 'home'} onClearAbout={onClearAbout} />
    </QueryClientProvider>,
  );
  return { onClearAbout, unmount };
}

beforeEach(() => {
  vi.clearAllMocks();
  api.getThread.mockResolvedValue({ messages: [] });
  api.markRead.mockResolvedValue(undefined);
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
    expect(api.markRead).not.toHaveBeenCalled();
  });

  it('shows a team reply as the Dravr team’s, and reading it marks it read', async () => {
    const reply: FeedbackMessage = {
      ...SENT,
      id: 'm-2',
      author: 'team',
      body: 'Fixed in tonight’s build.',
      created_at: '2026-10-28T16:00:00Z',
    };
    api.getThread.mockResolvedValue({ messages: [SENT, reply] });
    renderThread();

    const row = (await screen.findByText('Fixed in tonight’s build.')).closest('[data-group-start]');
    expect(row).not.toBeNull();
    // The welcome and the reply both read as the team; the replier is never named.
    expect(screen.getAllByText('Dravr team').length).toBeGreaterThanOrEqual(3);
    expect(screen.getAllByText(/Delivered/)).toHaveLength(1);
    await waitFor(() => expect(api.markRead).toHaveBeenCalledTimes(1));
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

  it('attaches a screenshot, shows it, and sends it alone as standard base64', async () => {
    const user = userEvent.setup();
    api.sendMessage.mockResolvedValue({ ...SENT, body: '', has_image: true });
    renderThread();

    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
    await user.upload(screen.getByTestId('feedback-image-input'), pngFile());
    expect(await screen.findByTestId('feedback-image-preview')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Send' })).toBeEnabled();

    await user.click(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => expect(api.sendMessage).toHaveBeenCalledTimes(1));
    const sent = api.sendMessage.mock.calls[0][0];
    expect(sent.body, 'a screenshot alone carries no words').toBe('');
    expect(sent.image?.media_type).toBe('image/png');
    expect(sent.image?.data, 'the bytes travel as base64, without the data: prefix').toBe(
      btoa(String.fromCharCode(0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a)),
    );
    await waitFor(() => expect(screen.queryByTestId('feedback-image-preview')).not.toBeInTheDocument());
  });

  it('lets the athlete take the screenshot back before sending', async () => {
    const user = userEvent.setup();
    renderThread();
    await user.upload(screen.getByTestId('feedback-image-input'), pngFile());
    await user.click(await screen.findByRole('button', { name: 'Remove the screenshot' }));
    expect(screen.queryByTestId('feedback-image-preview')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
  });

  it('refuses a screenshot over 5 MB or of another type, and attaches nothing', async () => {
    // Accept filtering off, so the composer's own check is what refuses the SVG.
    const user = userEvent.setup({ applyAccept: false });
    renderThread();

    await user.upload(screen.getByTestId('feedback-image-input'), pngFile('big.png', 5 * 1024 * 1024 + 1));
    expect(await screen.findByRole('alert')).toHaveTextContent('That image is over 5 MB. Pick a smaller one.');
    expect(screen.queryByTestId('feedback-image-preview')).not.toBeInTheDocument();

    const svg = new File(['<svg xmlns="http://www.w3.org/2000/svg"/>'], 'shot.svg', { type: 'image/svg+xml' });
    await user.upload(screen.getByTestId('feedback-image-input'), svg);
    expect(await screen.findByRole('alert')).toHaveTextContent('Attach a PNG, JPEG, WebP or GIF image.');
    expect(screen.queryByTestId('feedback-image-preview')).not.toBeInTheDocument();
    expect(api.sendMessage).not.toHaveBeenCalled();
  });

  it('takes a screenshot of exactly 5 MB', async () => {
    const user = userEvent.setup();
    renderThread();
    await user.upload(screen.getByTestId('feedback-image-input'), pngFile('edge.png', 5 * 1024 * 1024));
    expect(await screen.findByTestId('feedback-image-preview')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('stops saying a refusal once a message goes out', async () => {
    const user = userEvent.setup();
    api.sendMessage.mockResolvedValue(SENT);
    renderThread();

    await user.upload(screen.getByTestId('feedback-image-input'), pngFile('big.png', 5 * 1024 * 1024 + 1));
    expect(await screen.findByRole('alert')).toHaveTextContent('That image is over 5 MB. Pick a smaller one.');
    await user.type(screen.getByRole('textbox'), 'The week view skips my indoor ride.{Enter}');

    await waitFor(() => expect(api.sendMessage).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument());
  });

  it('shows the screenshot an athlete sent, read through the API client, and lets it go on leaving', async () => {
    api.getThread.mockResolvedValue({ messages: [{ ...SENT, has_image: true }] });
    api.getImage.mockResolvedValue(new Blob([new Uint8Array([0x89, 0x50])], { type: 'image/png' }));
    const { unmount } = renderThread();

    const image = await screen.findByRole('img', { name: 'Screenshot you sent' });
    expect(image).toHaveAttribute('src', 'blob:screenshot');
    expect(api.getImage).toHaveBeenCalledWith('m-1');

    unmount();
    expect(URL.revokeObjectURL, 'the object URL is released with the message').toHaveBeenCalledWith('blob:screenshot');
  });

  it('says so when a screenshot it sent cannot be loaded', async () => {
    api.getThread.mockResolvedValue({ messages: [{ ...SENT, body: '', has_image: true }] });
    api.getImage.mockRejectedValue(new Error('offline'));
    renderThread();

    expect(await screen.findByText("We couldn't load this screenshot.")).toBeInTheDocument();
    expect(screen.queryByRole('img', { name: 'Screenshot you sent' })).not.toBeInTheDocument();
  });

  it('says so when the browser cannot read the picked file, and attaches nothing', async () => {
    const user = userEvent.setup();
    // jsdom's FileReader reads every in-memory File it is handed; a browser
    // fails one whose bytes went away after it was picked. The read is failed
    // here the way a browser reports that: an error event, no result.
    const read = vi.spyOn(FileReader.prototype, 'readAsDataURL').mockImplementation(function (this: FileReader) {
      this.onerror?.(new ProgressEvent('error') as ProgressEvent<FileReader>);
    });
    renderThread();

    await user.upload(screen.getByTestId('feedback-image-input'), pngFile());
    expect(await screen.findByRole('alert')).toHaveTextContent("We couldn't read that file. Try another.");
    expect(screen.queryByTestId('feedback-image-preview')).not.toBeInTheDocument();
    read.mockRestore();
  });
});
