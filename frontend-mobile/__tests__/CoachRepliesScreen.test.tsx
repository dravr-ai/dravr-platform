// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The human coach's reply review on the phone over a mocked client — the list, the verdict chip, flag, correction, withheld, paging, empty and error
// ABOUTME: Pins what each write sends and that a refused write keeps the form open, the same states the web Replies page has (carnet#1304)

import React from 'react';
import { Alert } from 'react-native';
import Toast from 'react-native-toast-message';
import { AxiosError } from 'axios';
import { fireEvent, render, waitFor, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ClaimVerdict, CoachRepliesPage, CoachReply } from '@pierre/shared-types';

let mockParams: { athleteId?: string } = {};
jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useLocalSearchParams: () => mockParams,
  }),
);

const mockList = jest.fn<Promise<CoachRepliesPage>, [Record<string, unknown>]>();
const mockFlag = jest.fn();
const mockCorrect = jest.fn();
const mockWeekly = jest.fn();
jest.mock('../src/services/api', () => ({
  coachRepliesApi: {
    list: (query: Record<string, unknown>) => mockList(query),
    flag: (...args: unknown[]) => mockFlag(...args),
    correct: (...args: unknown[]) => mockCorrect(...args),
    weekly: (weeks?: number) => mockWeekly(weeks),
  },
}));

import { CoachRepliesScreen } from '../src/screens/coach-replies/CoachRepliesScreen';

function verdict(id: string, status: ClaimVerdict['status'], claim: string): ClaimVerdict {
  return {
    id,
    conversation_id: 'c1',
    message_id: 'm1',
    agent_id: null,
    claim_text: claim,
    category: 'athlete_data',
    status,
    evidence_strength: 'none',
    confidence: 0,
    layer_fired: 'athlete_data',
    explanation: null,
    evidence_refs: null,
    created_at: '2026-10-09T08:00:01Z',
  };
}

function reply(id: string, over: Partial<CoachReply> = {}): CoachReply {
  return {
    id,
    group_id: 'group-1',
    group_name: "Alex's block",
    athlete_user_id: 'user-alex',
    athlete_name: 'Alex',
    content: `Words of ${id}`,
    withheld: false,
    created_at: '2026-10-09T08:00:00Z',
    verdicts: [],
    unmatched_figures: 0,
    flags: [],
    corrections: [],
    ...over,
  };
}

const ATHLETES = [
  { user_id: 'user-alex', name: 'Alex', shares_with_coach: true },
  { user_id: 'user-ben', name: 'Ben', shares_with_coach: false },
];

function page(replies: CoachReply[], next: string | null = null): CoachRepliesPage {
  return { groups: [{ id: 'group-1', name: "Alex's block" }], athletes: ATHLETES, replies, next_before: next };
}

/** A refusal the server answered: it has a status, so it is not read as a dropped connection. */
function refused(): AxiosError {
  return new AxiosError('refused', 'ERR_BAD_REQUEST', undefined, undefined, {
    status: 400,
    statusText: 'Bad Request',
    data: {},
    headers: {},
    config: {} as never,
  });
}

function renderReview() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 }, mutations: { retry: false } } });
  return { client, ...render(<QueryClientProvider client={client}><CoachRepliesScreen /></QueryClientProvider>) };
}

beforeEach(() => {
  jest.clearAllMocks();
  mockParams = {};
  mockWeekly.mockResolvedValue({ weeks: [{ week_start: '2026-10-05', flags: 1, corrections: 0 }] });
  jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
});

describe('the list', () => {
  it('shows each reply\'s words, the unmatched-figure line and the checker\'s verdict chip', async () => {
    mockList.mockResolvedValue(
      page([
        reply('reply-1', {
          content: 'Your 2:41 marathon in May shows you are ready.',
          unmatched_figures: 2,
          verdicts: [verdict('v1', 'unverifiable', 'your 2:41 marathon in May'), verdict('v2', 'supported', 'ftp is 285 W')],
        }),
        reply('reply-2'),
      ]),
    );
    const screen = renderReview();
    const card = within(await screen.findByTestId('coach-reply-reply-1'));
    expect(card.getByTestId('coach-reply-content')).toHaveTextContent('Your 2:41 marathon in May shows you are ready.');
    expect(card.getByTestId('coach-reply-unmatched')).toHaveTextContent(/couldn't match to Alex's activities: 2/i);
    // The chat chip's own rollup: the supported claim leads, the abstention is counted.
    expect(card.getByTestId('coach-reply-verdict-chip')).toHaveTextContent(/1 supported.{1,5}1 unverifiable/);
    expect(card.getByTestId('coach-reply-verdict-chip').props.accessibilityLabel).toMatch(/^Claim verdicts: 1 supported/);
    expect(card.getAllByTestId('coach-reply-verdict')).toHaveLength(2);
    expect(card.getAllByTestId('coach-reply-verdict')[0]).toHaveTextContent(/ · your 2:41 marathon in May$/);
    expect(within(screen.getByTestId('coach-reply-reply-2')).queryByTestId('coach-reply-verdict-chip')).toBeNull();
  });

  it('counts the week\'s flags and corrections in the header', async () => {
    mockList.mockResolvedValue(page([reply('reply-1')]));
    const screen = renderReview();
    await waitFor(() => expect(screen.getByTestId('coach-replies-week')).toHaveTextContent('This week: 1 flagged · 0 corrected'));
    expect(mockWeekly).toHaveBeenCalledWith(1);
  });

  it('reads the first page newest first with the page size and no filter', async () => {
    mockList.mockResolvedValue(page([reply('reply-1')]));
    renderReview();
    await waitFor(() => expect(mockList).toHaveBeenCalledWith({ athlete_id: undefined, before: undefined, limit: 20 }));
  });

  it('opens narrowed to the athlete the link names, and re-reads for another', async () => {
    mockParams = { athleteId: 'user-alex' };
    mockList.mockResolvedValue(page([reply('reply-1')]));
    const screen = renderReview();
    await screen.findByTestId('coach-reply-reply-1');
    expect(mockList).toHaveBeenCalledWith({ athlete_id: 'user-alex', before: undefined, limit: 20 });
    fireEvent.press(screen.getByTestId('coach-replies-filter-all'));
    await waitFor(() => expect(mockList).toHaveBeenCalledWith({ athlete_id: undefined, before: undefined, limit: 20 }));
    expect(screen.getByTestId('coach-replies-filter-user-ben')).toHaveTextContent('Ben (not shared with you)');
  });

  it('pages older replies with the cursor and hides the button on the last page', async () => {
    mockList.mockImplementation(async (query) =>
      query.before === 'reply-2' ? page([reply('reply-1')]) : page([reply('reply-3'), reply('reply-2')], 'reply-2'),
    );
    const screen = renderReview();
    fireEvent.press(await screen.findByTestId('coach-replies-older'));
    await screen.findByTestId('coach-reply-reply-1');
    expect(mockList).toHaveBeenLastCalledWith({ athlete_id: undefined, before: 'reply-2', limit: 20 });
    expect(screen.getByTestId('coach-reply-reply-3')).toBeTruthy();
    await waitFor(() => expect(screen.queryByTestId('coach-replies-older')).toBeNull());
  });
});

describe('states', () => {
  it('shows a spinner while the first page loads', () => {
    mockList.mockReturnValue(new Promise(() => undefined));
    expect(renderReview().getByTestId('coach-replies-loading')).toBeTruthy();
  });

  it('says there are no replies yet', async () => {
    mockList.mockResolvedValue(page([]));
    const screen = renderReview();
    expect(await screen.findByTestId('coach-replies-empty')).toHaveTextContent(/No replies yet/);
  });

  it('says the caller does not run a coaching group yet', async () => {
    mockList.mockResolvedValue({ groups: [], athletes: [], replies: [], next_before: null });
    const screen = renderReview();
    expect(await screen.findByTestId('coach-replies-not-a-coach')).toHaveTextContent(/don't run a coaching group/);
  });

  it('says the replies could not load, and reads again on retry', async () => {
    mockList.mockRejectedValueOnce(new Error('down'));
    const screen = renderReview();
    expect(await screen.findByTestId('coach-replies-failed')).toHaveTextContent(/^Couldn't load the replies\./);
    mockList.mockResolvedValue(page([reply('reply-1')]));
    fireEvent.press(screen.getByTestId('coach-replies-retry'));
    await screen.findByTestId('coach-reply-reply-1');
  });

  it('withholds the words and the actions of a reply that cannot be shown', async () => {
    mockList.mockResolvedValue(page([reply('reply-1', { withheld: true, content: null })]));
    const screen = renderReview();
    const card = within(await screen.findByTestId('coach-reply-reply-1'));
    expect(card.getByTestId('coach-reply-withheld')).toHaveTextContent("This reply can't be shown here.");
    expect(card.queryByTestId('coach-reply-content')).toBeNull();
    expect(card.queryByTestId('coach-reply-flag-open')).toBeNull();
    expect(card.queryByTestId('coach-reply-correct-open')).toBeNull();
  });

  it('lists the flags and corrections the caller already made', async () => {
    mockList.mockResolvedValue(
      page([
        reply('reply-1', {
          flags: [{ id: 'f1', reply_entry_id: 'reply-1', reason: 'off_tone', note: 'too blunt', created_at: '2026-10-09T09:00:00Z' }],
          corrections: [
            { id: 'c1', reply_entry_id: 'reply-1', correction_entry_id: 'e9', content: 'It was 2:51.', created_at: '2026-10-09T10:00:00Z' },
          ],
        }),
      ]),
    );
    const screen = renderReview();
    const card = within(await screen.findByTestId('coach-reply-reply-1'));
    expect(card.getByTestId('coach-reply-flag')).toHaveTextContent('Flagged: Off-tone — too blunt');
    expect(card.getByTestId('coach-reply-correction')).toHaveTextContent(/Your correction.*It was 2:51\./);
  });
});

describe('flagging', () => {
  it('sends the reason and the note, closes the form and says so', async () => {
    mockList.mockResolvedValue(page([reply('reply-1')]));
    mockFlag.mockResolvedValue({ flag: {} });
    const screen = renderReview();
    fireEvent.press(await screen.findByTestId('coach-reply-flag-open'));
    fireEvent.press(screen.getByTestId('reply-flag-reason-unsafe'));
    fireEvent.changeText(screen.getByTestId('coach-reply-flag-note'), 'Tells him to skip rest');
    fireEvent.press(screen.getByTestId('coach-reply-flag-submit'));
    await waitFor(() => expect(mockFlag).toHaveBeenCalledWith('reply-1', 'unsafe', 'Tells him to skip rest'));
    await waitFor(() => expect(screen.queryByTestId('coach-reply-flag-form')).toBeNull());
    expect(Toast.show).toHaveBeenCalledWith({ type: 'info', text1: 'Reply flagged' });
  });

  it('defaults to "Wrong" and offers the three reasons', async () => {
    mockList.mockResolvedValue(page([reply('reply-1')]));
    mockFlag.mockResolvedValue({ flag: {} });
    const screen = renderReview();
    fireEvent.press(await screen.findByTestId('coach-reply-flag-open'));
    expect(within(screen.getByTestId('reply-flag-reasons')).getAllByRole('radio')).toHaveLength(3);
    fireEvent.press(screen.getByTestId('coach-reply-flag-submit'));
    await waitFor(() => expect(mockFlag).toHaveBeenCalledWith('reply-1', 'wrong', ''));
  });

  it('keeps the form and its note open, and says why, when the flag is refused', async () => {
    mockList.mockResolvedValue(page([reply('reply-1')]));
    mockFlag.mockRejectedValue(refused());
    const screen = renderReview();
    fireEvent.press(await screen.findByTestId('coach-reply-flag-open'));
    fireEvent.changeText(screen.getByTestId('coach-reply-flag-note'), 'keep me');
    fireEvent.press(screen.getByTestId('coach-reply-flag-submit'));
    await waitFor(() => expect(Alert.alert).toHaveBeenCalledWith('Error', "Couldn't flag the reply."));
    expect(screen.getByTestId('coach-reply-flag-note').props.value).toBe('keep me');
    expect(Toast.show).not.toHaveBeenCalled();
  });

  it('cancels without sending anything', async () => {
    mockList.mockResolvedValue(page([reply('reply-1')]));
    const screen = renderReview();
    fireEvent.press(await screen.findByTestId('coach-reply-flag-open'));
    fireEvent.press(screen.getByTestId('coach-reply-flag-cancel'));
    expect(screen.queryByTestId('coach-reply-flag-form')).toBeNull();
    expect(mockFlag).not.toHaveBeenCalled();
  });
});

describe('correcting', () => {
  it('cannot post an empty correction', async () => {
    mockList.mockResolvedValue(page([reply('reply-1')]));
    const screen = renderReview();
    fireEvent.press(await screen.findByTestId('coach-reply-correct-open'));
    fireEvent.changeText(screen.getByTestId('coach-reply-correction-input'), '   ');
    fireEvent.press(screen.getByTestId('coach-reply-correct-submit'));
    expect(mockCorrect).not.toHaveBeenCalled();
  });

  it('posts the trimmed correction into the thread, closes the form and re-reads the review', async () => {
    mockList.mockResolvedValue(page([reply('reply-1')]));
    mockCorrect.mockResolvedValue({ correction: {}, posted_to_chat: false });
    const screen = renderReview();
    fireEvent.press(await screen.findByTestId('coach-reply-correct-open'));
    expect(screen.getByTestId('coach-reply-correction-input').props.placeholder).toBe('What should Alex know instead?');
    fireEvent.changeText(screen.getByTestId('coach-reply-correction-input'), '  It was 2:51.  ');
    const readsBefore = mockList.mock.calls.length;
    fireEvent.press(screen.getByTestId('coach-reply-correct-submit'));
    await waitFor(() => expect(mockCorrect).toHaveBeenCalledWith('reply-1', 'It was 2:51.'));
    await waitFor(() => expect(screen.queryByTestId('coach-reply-correct-form')).toBeNull());
    expect(Toast.show).toHaveBeenCalledWith({ type: 'info', text1: 'Correction posted in the thread' });
    await waitFor(() => expect(mockList.mock.calls.length).toBeGreaterThan(readsBefore));
  });

  it('keeps the correction open, and says why, when it is refused', async () => {
    mockList.mockResolvedValue(page([reply('reply-1')]));
    mockCorrect.mockRejectedValue(refused());
    const screen = renderReview();
    fireEvent.press(await screen.findByTestId('coach-reply-correct-open'));
    fireEvent.changeText(screen.getByTestId('coach-reply-correction-input'), 'Ran 2:51');
    fireEvent.press(screen.getByTestId('coach-reply-correct-submit'));
    await waitFor(() => expect(Alert.alert).toHaveBeenCalledWith('Error', "Couldn't post the correction."));
    expect(screen.getByTestId('coach-reply-correction-input').props.value).toBe('Ran 2:51');
  });
});
