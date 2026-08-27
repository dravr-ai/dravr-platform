// ABOUTME: Unit tests for useMessages hook
// ABOUTME: Tests message state management, sending, setMessages/setIsSending exposure

import React from 'react';
import { renderHook as rtlRenderHook, act } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

// Mock API service
const mockGetConversationMessages = jest.fn();
const mockSendTurn = jest.fn();
const mockSubmitMessageFeedback = jest.fn();
const mockDeleteMessageFeedback = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversationMessages: (...args: unknown[]) => mockGetConversationMessages(...args),
    sendTurn: (...args: unknown[]) => mockSendTurn(...args),
    submitMessageFeedback: (...args: unknown[]) => mockSubmitMessageFeedback(...args),
    deleteMessageFeedback: (...args: unknown[]) => mockDeleteMessageFeedback(...args),
  },
}));

jest.mock('@pierre/chat-utils', () => ({
  // loadMessages now drops tool_call/tool_result plumbing rows via this helper;
  // mirror the real implementation so the hook under test behaves identically.
  filterDisplayMessages: (messages: { role: string }[]) =>
    messages.filter((m) => m.role !== 'tool_call' && m.role !== 'tool_result'),
  // The real mapping, not a stub: the progress line the athlete reads is the
  // point of the strip, and a stubbed mapper would let a broken one pass.
  statusTextForProgress: jest.requireActual('@pierre/chat-utils').statusTextForProgress,
}));

import { useMessages } from '../src/screens/chat/useMessages';
import type { Message } from '../src/types';

/**
 * The hook invalidates the conversation-list query after a turn, so it needs
 * a client in scope — the same one the app's QueryProvider supplies.
 */
function renderHook<T>(hook: () => T) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return rtlRenderHook(hook, {
    wrapper: ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
}

const createMockMessage = (overrides: Partial<Message> = {}): Message => ({
  id: `msg-${Date.now()}`,
  role: 'assistant',
  content: 'Test message',
  created_at: new Date().toISOString(),
  ...overrides,
});

describe('useMessages', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  describe('initial state', () => {
    it('should start with empty messages', () => {
      const { result } = renderHook(() => useMessages());
      expect(result.current.messages).toEqual([]);
    });

    it('should start with isSending false', () => {
      const { result } = renderHook(() => useMessages());
      expect(result.current.isSending).toBe(false);
    });
  });

  describe('setMessages', () => {
    it('should be exposed and functional', () => {
      const { result } = renderHook(() => useMessages());

      expect(result.current.setMessages).toBeDefined();
      expect(typeof result.current.setMessages).toBe('function');
    });

    it('should set messages with direct value', () => {
      const { result } = renderHook(() => useMessages());

      const messages = [
        createMockMessage({ id: 'msg-1', content: 'Hello' }),
        createMockMessage({ id: 'msg-2', content: 'World' }),
      ];

      act(() => {
        result.current.setMessages(messages);
      });

      expect(result.current.messages).toHaveLength(2);
      expect(result.current.messages[0].content).toBe('Hello');
      expect(result.current.messages[1].content).toBe('World');
    });

    it('should set messages with function updater', () => {
      const { result } = renderHook(() => useMessages());

      // Set initial messages
      const initialMsg = createMockMessage({ id: 'msg-1', content: 'First' });
      act(() => {
        result.current.setMessages([initialMsg]);
      });

      // Update using function updater (as useCoachSelection does)
      const newMsg = createMockMessage({ id: 'msg-2', content: 'Second' });
      act(() => {
        result.current.setMessages(prev => [...prev, newMsg]);
      });

      expect(result.current.messages).toHaveLength(2);
      expect(result.current.messages[0].content).toBe('First');
      expect(result.current.messages[1].content).toBe('Second');
    });

    it('should support the coach conversation pattern: set then update', () => {
      const { result } = renderHook(() => useMessages());

      // Step 1: Coach sets initial user message (direct value)
      const tempUserMsg = createMockMessage({
        id: 'temp-123',
        role: 'user',
        content: 'Analyze my running data',
      });
      act(() => {
        result.current.setMessages([tempUserMsg]);
      });

      expect(result.current.messages).toHaveLength(1);
      expect(result.current.messages[0].role).toBe('user');

      // Step 2: Coach replaces temp message with API response (function updater)
      const realUserMsg = createMockMessage({
        id: 'user-456',
        role: 'user',
        content: 'Analyze my running data',
      });
      const assistantMsg = createMockMessage({
        id: 'asst-789',
        role: 'assistant',
        content: 'Your VO2max is improving!',
      });
      act(() => {
        result.current.setMessages(prev => {
          const filtered = prev.filter(m => m.id !== 'temp-123');
          return [...filtered, realUserMsg, assistantMsg];
        });
      });

      expect(result.current.messages).toHaveLength(2);
      expect(result.current.messages[0].id).toBe('user-456');
      expect(result.current.messages[1].id).toBe('asst-789');
      expect(result.current.messages[1].content).toBe('Your VO2max is improving!');
    });
  });

  describe('setIsSending', () => {
    it('should be exposed and functional', () => {
      const { result } = renderHook(() => useMessages());

      expect(result.current.setIsSending).toBeDefined();
      expect(typeof result.current.setIsSending).toBe('function');
    });

    it('should set isSending to true', () => {
      const { result } = renderHook(() => useMessages());

      act(() => {
        result.current.setIsSending(true);
      });

      expect(result.current.isSending).toBe(true);
    });

    it('should set isSending back to false', () => {
      const { result } = renderHook(() => useMessages());

      act(() => {
        result.current.setIsSending(true);
      });
      expect(result.current.isSending).toBe(true);

      act(() => {
        result.current.setIsSending(false);
      });
      expect(result.current.isSending).toBe(false);
    });
  });

  describe('clearMessages', () => {
    it('should clear all messages', () => {
      const { result } = renderHook(() => useMessages());

      act(() => {
        result.current.setMessages([
          createMockMessage({ id: 'msg-1' }),
          createMockMessage({ id: 'msg-2' }),
        ]);
      });
      expect(result.current.messages).toHaveLength(2);

      act(() => {
        result.current.clearMessages();
      });
      expect(result.current.messages).toEqual([]);
    });
  });

  describe('sendMessage', () => {
    it('should add temp message, call API, and update with response', async () => {
      const apiResponse = {
        user_message: { id: 'user-1', role: 'user', content: 'Hello', created_at: '2024-01-01T00:00:00Z' },
        assistant: {
          message: { id: 'asst-1', role: 'assistant', content: 'Hi there!', created_at: '2024-01-01T00:00:01Z' },
          blocks: [],
          finish_reason: 'stop',
        },
        telemetry: {
          model: 'gemini-2.0-flash',
          provider_name: 'gemini',
          tool_calls_count: 0,
          tools_called: [],
          execution_time_ms: 2000,
        },
      };
      // `sendTurn` reports the finished turn through `onDone` rather than
      // returning it, because the same method streams for the web client.
      mockSendTurn.mockImplementation(
        (
          _conversationId: string,
          _content: string,
          options: {
            onProgress?: (progress: unknown) => void;
            onBlock?: (block: unknown) => void;
            onDone?: (turn: unknown) => void;
          },
        ) => {
          // Faithful to the real transport: progress as the turn advances,
          // then every block in order, then the turn.
          options.onProgress?.({
            kind: 'stage',
            id: 'dispatch',
            title: 'dispatch',
            status: 'started',
          });
          for (const block of apiResponse.assistant.blocks) options.onBlock?.(block);
          options.onDone?.(apiResponse);
          return Promise.resolve();
        },
      );

      const { result } = renderHook(() => useMessages());

      await act(async () => {
        await result.current.sendTurn('conv-1', 'Hello');
      });

      // Progress rides the turn's own body now — no run id, no second
      // subscription. What the hook threads in instead is the idle watch's
      // abort signal, so an abandoned stream can be dropped.
      expect(mockSendTurn).toHaveBeenCalledWith(
        'conv-1',
        'Hello',
        expect.objectContaining({
          signal: expect.any(AbortSignal),
          onProgress: expect.any(Function),
        }),
      );
      const sentOptions = mockSendTurn.mock.calls[0][2] as Record<string, unknown>;
      expect(sentOptions).not.toHaveProperty('aguiRunId');
      expect(result.current.messages).toHaveLength(2);
      expect(result.current.messages[0].role).toBe('user');
      expect(result.current.messages[1].role).toBe('assistant');
      expect(result.current.isSending).toBe(false);
      // The strip collapses once the reply is the source of truth.
      expect(result.current.progressText).toBeNull();
    });

    it('shows what the turn is doing while it is still in flight, then clears it', async () => {
      // The progress strip is the whole reason the AG-UI subscription existed.
      // It is now fed by the turn's own body, so the text must appear while
      // the send is still running — asserting it after the turn lands would
      // pass against a hook that never rendered anything.
      let release: (() => void) | null = null;
      const inFlight = new Promise<void>((resolve) => {
        release = resolve;
      });
      mockSendTurn.mockImplementation(
        async (
          _conversationId: string,
          _content: string,
          options: {
            onProgress?: (progress: unknown) => void;
            onDone?: (turn: unknown) => void;
          },
        ) => {
          options.onProgress?.({
            kind: 'stage',
            id: 'prompt_assembly',
            title: 'prompt_assembly',
            status: 'started',
          });
          options.onProgress?.({
            kind: 'tool',
            id: 'call-1',
            title: 'get_activities',
            status: 'InProgress',
          });
          await inFlight;
          options.onDone?.({
            user_message: { id: 'u', role: 'user', content: 'Hello', created_at: 'now' },
            assistant: {
              message: { id: 'a', role: 'assistant', content: 'Voila.', created_at: 'now' },
              blocks: [],
              finish_reason: 'stop',
            },
            telemetry: {
              model: 'mock',
              provider_name: 'mock',
              tool_calls_count: 1,
              tools_called: ['get_activities'],
              execution_time_ms: 10,
            },
          });
        },
      );

      const { result } = renderHook(() => useMessages());

      let sending: Promise<void>;
      await act(async () => {
        sending = result.current.sendTurn('conv-1', 'Hello');
        await Promise.resolve();
      });

      // The latest progress event wins, in the vocabulary every surface shares.
      expect(result.current.progressText).toBe('calling get_activities…');

      await act(async () => {
        release?.();
        await sending;
      });

      expect(result.current.progressText).toBeNull();
      expect(result.current.messages.some((m) => m.role === 'assistant')).toBe(true);
    });

    it('should handle API error and show error message', async () => {
      mockSendTurn.mockImplementation(
        (_conversationId: string, _content: string, options: { onError?: (error: Error) => void }) => {
          options.onError?.(new Error('Network timeout'));
          return Promise.resolve();
        },
      );

      const { result } = renderHook(() => useMessages());

      await act(async () => {
        await result.current.sendTurn('conv-1', 'Hello');
      });

      // Should have user message and error message
      expect(result.current.messages).toHaveLength(2);
      const errorMsg = result.current.messages[1];
      expect(errorMsg.role).toBe('assistant');
      expect(errorMsg.isError).toBe(true);
      expect(errorMsg.content).toContain('Network timeout');
      expect(result.current.isSending).toBe(false);
    });

    it('should not send when already sending', async () => {
      mockSendTurn.mockImplementation(() => new Promise(() => {})); // Never resolves

      const { result } = renderHook(() => useMessages());

      // Start first send (won't resolve)
      act(() => {
        result.current.sendTurn('conv-1', 'First');
      });

      // Try to send again while first is pending
      await act(async () => {
        await result.current.sendTurn('conv-1', 'Second');
      });

      // Should only have been called once
      expect(mockSendTurn).toHaveBeenCalledTimes(1);
    });
  });

  describe('message feedback', () => {
    it('thumbs up optimistically sets the rating and persists it', async () => {
      mockSubmitMessageFeedback.mockResolvedValue({ message_id: 'asst-1', rating: 'up' });
      const { result } = renderHook(() => useMessages());

      await act(async () => {
        await result.current.handleThumbsUp('asst-1', 'conv-1');
      });

      expect(result.current.messageFeedback['asst-1']).toBe('up');
      expect(mockSubmitMessageFeedback).toHaveBeenCalledWith('conv-1', 'asst-1', 'up');
    });

    it('clicking the active rating again toggles it off via DELETE', async () => {
      mockSubmitMessageFeedback.mockResolvedValue({ message_id: 'asst-1', rating: 'up' });
      mockDeleteMessageFeedback.mockResolvedValue(undefined);
      const { result } = renderHook(() => useMessages());

      await act(async () => {
        await result.current.handleThumbsUp('asst-1', 'conv-1');
      });
      expect(result.current.messageFeedback['asst-1']).toBe('up');

      await act(async () => {
        await result.current.handleThumbsUp('asst-1', 'conv-1');
      });

      expect(result.current.messageFeedback['asst-1']).toBeNull();
      expect(mockDeleteMessageFeedback).toHaveBeenCalledWith('conv-1', 'asst-1');
    });

    it('submitting a reason persists rating=down with the comment', async () => {
      mockSubmitMessageFeedback.mockResolvedValue({
        message_id: 'asst-1',
        rating: 'down',
        comment: 'too vague',
      });
      const { result } = renderHook(() => useMessages());

      await act(async () => {
        await result.current.submitFeedbackReason('asst-1', 'conv-1', '  too vague  ');
      });

      expect(result.current.messageFeedbackComment['asst-1']).toBe('too vague');
      expect(mockSubmitMessageFeedback).toHaveBeenCalledWith('conv-1', 'asst-1', 'down', 'too vague');
    });

    it('reverts the optimistic rating when the API call fails', async () => {
      mockSubmitMessageFeedback.mockRejectedValue(new Error('offline'));
      const { result } = renderHook(() => useMessages());

      await act(async () => {
        await result.current.handleThumbsUp('asst-1', 'conv-1');
      });

      expect(result.current.messageFeedback['asst-1']).toBeNull();
      expect(result.current.error).toBe('offline');
    });

    it('hydrates feedback state from the messages-list response on load', async () => {
      mockGetConversationMessages.mockResolvedValue({
        messages: [createMockMessage({ id: 'asst-1' })],
        feedback: [{ message_id: 'asst-1', rating: 'down', comment: 'missing detail' }],
      });
      const { result } = renderHook(() => useMessages());

      await act(async () => {
        await result.current.loadMessages('conv-1');
      });

      expect(result.current.messageFeedback['asst-1']).toBe('down');
      expect(result.current.messageFeedbackComment['asst-1']).toBe('missing detail');
    });
  });

  describe('loadMessages tool-row filtering', () => {
    it('drops tool_call / tool_result plumbing rows from the loaded thread', async () => {
      mockGetConversationMessages.mockResolvedValue({
        messages: [
          createMockMessage({ id: 'u-1', role: 'user', content: 'how was my run?' }),
          createMockMessage({ id: 'tc-1', role: 'tool_call', content: '<tool_call>get_activities</tool_call>' }),
          createMockMessage({ id: 'tr-1', role: 'tool_result', content: '<tool_result>{"d":5000}</tool_result>' }),
          createMockMessage({ id: 'a-1', role: 'assistant', content: 'Your run was 5km.' }),
        ],
        feedback: [],
      });
      const { result } = renderHook(() => useMessages());

      await act(async () => {
        await result.current.loadMessages('conv-1');
      });

      const roles = result.current.messages.map((m) => m.role);
      expect(roles).toEqual(['user', 'assistant']);
      // No raw scaffolding survives into the rendered thread.
      expect(result.current.messages.some((m) => m.content.includes('<tool_'))).toBe(false);
    });
  });
});
