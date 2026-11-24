// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The chat screen's open conversation is a row of the one React Query list, never a copy of it
// ABOUTME: Red if the open row stops following the list, a new thread is not found at once, or a first read is skipped wrongly

import React from 'react';
import { Alert } from 'react-native';
import { renderHook, act, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';

const mockGetConversations = jest.fn();
const mockCreateConversation = jest.fn();
const mockUpdateConversation = jest.fn();
const mockDeleteConversation = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversations: (...args: unknown[]) => mockGetConversations(...args),
    createConversation: (...args: unknown[]) => mockCreateConversation(...args),
    updateConversation: (...args: unknown[]) => mockUpdateConversation(...args),
    deleteConversation: (...args: unknown[]) => mockDeleteConversation(...args),
  },
}));

import { useOpenConversation } from '../src/screens/chat/useOpenConversation';
import { useConversationRows } from '../src/screens/conversations/useConversationList';

function conv(id: string, title = `Chat ${id}`) {
  return {
    id,
    title,
    agent_id: null,
    message_count: 1,
    unread_count: 0,
    created_at: '2026-04-10T10:00:00Z',
    updated_at: '2026-04-10T10:00:00Z',
  };
}

// Every hook a test mounted and the client under it: unmounted, then emptied,
// so no cache timer outlives the test.
const mounted: Array<{ unmount: () => void; client: QueryClient }> = [];

function render() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const view = renderHook(
    () => ({ open: useOpenConversation(), list: useConversationRows() }),
    {
      wrapper: ({ children }: { children: React.ReactNode }) => (
        <QueryClientProvider client={client}>{children}</QueryClientProvider>
      ),
    },
  );
  mounted.push({ unmount: view.unmount, client });
  return { ...view, client };
}

describe('useOpenConversation', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetConversations.mockResolvedValue({ conversations: [conv('a'), conv('b')], total: 2 });
  });
  afterEach(() => {
    for (const { unmount, client } of mounted.splice(0)) {
      unmount();
      client.clear();
    }
  });

  it('reads the open row out of the one list the list screen reads', async () => {
    const { result } = render();
    await waitFor(() => expect(result.current.open.isLoading).toBe(false));

    act(() => result.current.open.openConversation('b'));

    await waitFor(() => expect(result.current.open.currentConversation?.id).toBe('b'));
    // One holder: the hook's rows are the list's rows, fed by one request.
    expect(result.current.open.conversations).toEqual(result.current.list.conversations);
    expect(mockGetConversations).toHaveBeenCalledTimes(1);
  });

  it('pages on until it finds a thread opened by id below the first page', async () => {
    mockGetConversations.mockImplementation((_limit: number, offset: number) =>
      Promise.resolve(
        offset === 0
          ? { conversations: [conv('a'), conv('b')], total: 3 }
          : { conversations: [conv('deep')], total: 3 },
      ),
    );
    const { result } = render();

    act(() => result.current.open.openConversation('deep'));

    await waitFor(() => expect(result.current.open.currentConversation?.id).toBe('deep'));
    expect(mockGetConversations).toHaveBeenCalledWith(50, 2);
  });

  it('opens a new thread at once, from the row the create answered', async () => {
    // The list read that follows a create is held back: the row must already
    // be the open one before it answers.
    const { result } = render();
    await waitFor(() => expect(result.current.open.isLoading).toBe(false));
    let answerListRead: (page: unknown) => void = () => undefined;
    mockGetConversations.mockReturnValue(new Promise((resolve) => { answerListRead = resolve; }));
    mockCreateConversation.mockResolvedValue(conv('new', 'Tempo Agent'));

    let id = '';
    await act(async () => {
      id = await result.current.open.createConversation({ agent_id: 'a1' });
    });

    expect(id).toBe('new');
    await waitFor(() => expect(result.current.open.currentConversation?.title).toBe('Tempo Agent'));
    // The list screen draws the same new row, first.
    expect(result.current.list.conversations[0].id).toBe('new');

    await act(async () => {
      answerListRead({ conversations: [conv('new', 'Tempo Agent'), conv('a'), conv('b')], total: 3 });
    });
    expect(result.current.open.currentConversation?.id).toBe('new');
  });

  // The server posts the agent's welcome while creating the thread, so
  // skipping the first read would open it blank.
  it('reads an agent-bound thread, and one created without an agent, on open', async () => {
    mockCreateConversation.mockResolvedValue(conv('conv-agent'));
    const { result } = render();

    await act(async () => {
      await result.current.open.createConversation({ agent_id: 'a1' });
    });
    expect(result.current.open.justCreatedConversationRef.current).toBeNull();

    await act(async () => {
      await result.current.open.createConversation({});
    });
    expect(result.current.open.justCreatedConversationRef.current).toBeNull();
  });

  // A thread created for the line being sent gets no welcome: it is empty,
  // and a read still in flight when the line lands would wipe it.
  it('skips the first read of a thread the athlete speaks first in', async () => {
    mockCreateConversation.mockResolvedValue(conv('conv-first'));
    const { result } = render();

    await act(async () => {
      await result.current.open.createConversation({ athlete_speaks_first: true });
    });

    expect(mockCreateConversation).toHaveBeenCalledWith({ athlete_speaks_first: true });
    expect(result.current.open.justCreatedConversationRef.current).toBe('conv-first');
  });

  it('renames through the list, so the open row and the list row are one', async () => {
    mockUpdateConversation.mockResolvedValue({ ...conv('a', 'Renamed'), updated_at: '2026-04-11T10:00:00Z' });
    const { result } = render();
    await waitFor(() => expect(result.current.open.isLoading).toBe(false));
    act(() => result.current.open.openConversation('a'));
    mockGetConversations.mockResolvedValue({ conversations: [conv('a', 'Renamed'), conv('b')], total: 2 });

    await act(async () => {
      await result.current.open.renameConversation('a', 'Renamed');
    });

    expect(mockUpdateConversation).toHaveBeenCalledWith('a', { title: 'Renamed' });
    await waitFor(() => expect(result.current.open.currentConversation?.title).toBe('Renamed'));
    expect(result.current.list.conversations.find((row) => row.id === 'a')?.title).toBe('Renamed');
  });

  it('says a failed rename and leaves the row as it was', async () => {
    const alert = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    mockUpdateConversation.mockRejectedValue(new Error('offline'));
    const logged = jest.spyOn(console, 'error').mockImplementation(() => undefined);
    const { result } = render();
    await waitFor(() => expect(result.current.open.isLoading).toBe(false));
    act(() => result.current.open.openConversation('a'));

    await act(async () => {
      await result.current.open.renameConversation('a', 'Renamed');
    });

    expect(alert).toHaveBeenCalledTimes(1);
    expect(result.current.open.currentConversation?.title).toBe('Chat a');
    alert.mockRestore();
    logged.mockRestore();
  });

  it('deletes through the list: the row leaves it, the thread closes, the personal thread is re-read', async () => {
    mockDeleteConversation.mockResolvedValue(undefined);
    const { result, client } = render();
    const invalidate = jest.spyOn(client, 'invalidateQueries');
    await waitFor(() => expect(result.current.open.isLoading).toBe(false));
    act(() => result.current.open.openConversation('a'));
    mockGetConversations.mockResolvedValue({ conversations: [conv('b')], total: 1 });

    await act(async () => {
      await result.current.open.deleteConversation('a');
    });

    expect(mockDeleteConversation).toHaveBeenCalledWith('a');
    await waitFor(() => expect(result.current.open.openId).toBeNull());
    expect(result.current.open.currentConversation).toBeNull();
    await waitFor(() =>
      expect(result.current.list.conversations.map((row) => row.id)).toEqual(['b']),
    );
    expect(invalidate).toHaveBeenCalledWith({ queryKey: QUERY_KEYS.chat.personal() });
  });

  it('moves to a thread the server forged once the list has read it', async () => {
    const { result } = render();
    await waitFor(() => expect(result.current.open.isLoading).toBe(false));
    act(() => result.current.open.openConversation('a'));
    mockGetConversations.mockResolvedValue({ conversations: [conv('fresh'), conv('a'), conv('b')], total: 3 });

    let opened = false;
    await act(async () => {
      opened = await result.current.open.switchToConversation('fresh');
    });

    expect(opened).toBe(true);
    await waitFor(() => expect(result.current.open.currentConversation?.id).toBe('fresh'));
  });

  it('stays where it is when the list does not hold the thread a turn moved to', async () => {
    const { result } = render();
    await waitFor(() => expect(result.current.open.isLoading).toBe(false));
    act(() => result.current.open.openConversation('a'));

    let opened = true;
    await act(async () => {
      opened = await result.current.open.switchToConversation('gone');
    });

    expect(opened).toBe(false);
    expect(result.current.open.currentConversation?.id).toBe('a');
  });
});
