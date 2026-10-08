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
const mockGetConversationVerdicts = jest.fn().mockResolvedValue({ verdicts: [] });

jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversationMessages: (...args: unknown[]) => mockGetConversationMessages(...args),
    sendTurn: (...args: unknown[]) => mockSendTurn(...args),
    submitMessageFeedback: (...args: unknown[]) => mockSubmitMessageFeedback(...args),
    deleteMessageFeedback: (...args: unknown[]) => mockDeleteMessageFeedback(...args),
    getConversationVerdicts: (...args: unknown[]) => mockGetConversationVerdicts(...args),
  },
}));

jest.mock('@pierre/chat-utils', () => ({
  // loadMessages now drops tool_call/tool_result plumbing rows via this helper;
  // mirror the real implementation so the hook under test behaves identically.
  filterDisplayMessages: (messages: { role: string }[]) =>
    messages.filter((m) => m.role !== 'tool_call' && m.role !== 'tool_result'),
  // The real mapping, not a stub: the progress line the athlete reads is the
  // point of the strip, and a stubbed mapper would let a broken one pass.
  statusForProgress: jest.requireActual('@pierre/chat-utils').statusForProgress,
  // The real lost-turn reducer: every send and every read goes through it,
  // and a stub would decide nothing about when a failure's note stands.
  readLostTurn: jest.requireActual('@pierre/chat-utils').readLostTurn,
  reduceLostTurn: jest.requireActual('@pierre/chat-utils').reduceLostTurn,
}));

import { useMessages } from '../src/screens/chat/useMessages';
import type { Message } from '../src/types';
import { networkFailure } from '../integration/app/helpers/apiRefusal';

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

      // Step 1: Agent sets initial user message (direct value)
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

      // Step 2: Agent replaces temp message with API response (function updater)
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

    // carnet#828: a starter sends its opaque postback and shows its label;
    // the athlete's line becomes what the server wrote, unless it wrote none.
    describe('a tapped suggestion', () => {
      const LABEL = 'How many gels during a marathon?';
      const turnWith = (userContent: string) => ({
        user_message: { id: 'user-1', role: 'user', content: userContent, created_at: '2024-01-01T00:00:00Z' },
        assistant: {
          message: { id: 'asst-1', role: 'assistant', content: 'About one every 30 minutes.', created_at: '2024-01-01T00:00:01Z' },
          blocks: [],
          finish_reason: 'stop',
        },
        telemetry: { model: 'm', provider_name: 'p', tool_calls_count: 0, tools_called: [], execution_time_ms: 1 },
      });

      it('sends the postback, shows the label, then the echoed line', async () => {
        const ECHO = 'How many gels during a marathon, and when?';
        let finish: (() => void) | undefined;
        mockSendTurn.mockImplementation(
          (_c: string, _content: string, options: { onDone?: (turn: unknown) => void }) =>
            new Promise<void>((resolve) => {
              finish = () => {
                options.onDone?.(turnWith(ECHO));
                resolve();
              };
            }),
        );
        const { result } = renderHook(() => useMessages());

        let sending: Promise<unknown> = Promise.resolve();
        await act(async () => {
          sending = result.current.sendTurn('conv-1', 'ex:2:2', { display: LABEL });
        });
        expect(mockSendTurn.mock.calls[0][1]).toBe('ex:2:2');
        expect(result.current.messages[0].content).toBe(LABEL);

        await act(async () => {
          finish?.();
          await sending;
        });
        expect(result.current.messages[0]).toMatchObject({ id: 'user-1', content: ECHO });
      });

      it('keeps the label when the server echoes nothing', async () => {
        mockSendTurn.mockImplementation(
          (_c: string, _content: string, options: { onDone?: (turn: unknown) => void }) => {
            options.onDone?.(turnWith(''));
            return Promise.resolve();
          },
        );
        const { result } = renderHook(() => useMessages());

        await act(async () => {
          await result.current.sendTurn('conv-1', 'ex:0:9', { display: LABEL });
        });

        expect(result.current.messages[0].content).toBe(LABEL);
        expect(result.current.messages.map(m => m.content)).not.toContain('ex:0:9');
      });

      it('retries a failed tap with its postback, never the label it shows', async () => {
        mockSendTurn.mockImplementationOnce(
          (_c: string, _content: string, options: { onError?: (error: Error) => void }) => {
            options.onError?.(new TypeError('Network request failed'));
            return Promise.resolve();
          },
        );
        const { result } = renderHook(() => useMessages());

        await act(async () => {
          await result.current.sendTurn('conv-1', 'uc:1:last_workout', { display: LABEL });
        });
        const [question, failure] = result.current.messages;
        expect(question.content).toBe(LABEL);
        expect(failure.isError).toBe(true);

        mockSendTurn.mockImplementationOnce(
          (_c: string, _content: string, options: { onDone?: (turn: unknown) => void }) => {
            options.onDone?.(turnWith('Look at my last workout.'));
            return Promise.resolve();
          },
        );
        await act(async () => {
          await result.current.retryMessage(failure.id, 'conv-1');
        });

        expect(mockSendTurn.mock.calls[1][1]).toBe('uc:1:last_workout');
        expect(result.current.messages.map(m => m.content)).not.toContain('uc:1:last_workout');
      });

      it('passes how the message was produced on to the request', async () => {
        mockSendTurn.mockResolvedValue(undefined);
        const { result } = renderHook(() => useMessages());

        await act(async () => {
          await result.current.sendTurn('conv-1', 'How was my pacing?', { origin: 'chip' });
        });

        expect(mockSendTurn.mock.calls[0][2]).toMatchObject({ origin: 'chip' });
      });
    });

    it('reads the conversation\'s verdict rows once the turn completes', async () => {
      // The stream's `verdicts` block names only flagged claims, so a reply
      // whose claims all held streams none; its chip rail comes from the rows.
      const supported = {
        id: 'v1',
        conversation_id: 'conv-1',
        message_id: 'asst-1',
        agent_id: null,
        claim_text: 'Your threshold pace is 4:10/km.',
        category: 'training_prescription',
        status: 'supported',
        evidence_strength: 'strong',
        confidence: 0.9,
        layer_fired: 'deterministic',
        explanation: null,
        evidence_refs: null,
        created_at: '2024-01-01T00:00:02Z',
      };
      mockGetConversationVerdicts.mockResolvedValueOnce({ verdicts: [supported] });
      mockSendTurn.mockImplementation(
        (_conversationId: string, _content: string, options: { onDone?: (turn: unknown) => void }) => {
          options.onDone?.({
            user_message: { id: 'user-1', role: 'user', content: 'Hello', created_at: '2024-01-01T00:00:00Z' },
            assistant: {
              message: { id: 'asst-1', role: 'assistant', content: 'Hold 4:10/km.', created_at: '2024-01-01T00:00:01Z' },
              blocks: [],
              finish_reason: 'stop',
            },
            telemetry: { model: 'm', provider_name: 'p', tool_calls_count: 0, tools_called: [], execution_time_ms: 1 },
          });
          return Promise.resolve();
        },
      );

      const { result } = renderHook(() => useMessages());

      await act(async () => {
        await result.current.sendTurn('conv-1', 'Hello');
      });

      expect(mockGetConversationVerdicts).toHaveBeenCalledWith('conv-1');
      expect(result.current.verdicts).toEqual([supported]);
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

      let sending: Promise<string | null>;
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
          options.onError?.(new TypeError('Network request failed'));
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
      // Worded from the catalogue, never the transport's own English.
      expect(errorMsg.content).toContain('Network error. Check your connection.');
      expect(errorMsg.content).not.toContain('Network request failed');
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
      mockSubmitMessageFeedback.mockRejectedValue(networkFailure());
      const { result } = renderHook(() => useMessages());

      await act(async () => {
        await result.current.handleThumbsUp('asst-1', 'conv-1');
      });

      expect(result.current.messageFeedback['asst-1']).toBeNull();
      expect(result.current.error).toBe('Network error. Check your connection.');
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

describe('useMessages conversation rotation', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  /** Finish the next turn with an envelope that names another conversation. */
  function answerWithRotation(rotatedTo?: string) {
    mockSendTurn.mockImplementation(
      async (
        _conversationId: string,
        _text: string,
        options: { onDone?: (turn: Record<string, unknown>) => void },
      ) => {
        options.onDone?.({
          turn_id: 't1',
          user_message: { id: 'u1', role: 'user', content: '/reset', created_at: '2026-09-02T10:00:00Z' },
          assistant: {
            message: { id: 'a1', role: 'assistant', content: 'New conversation started.', created_at: '2026-09-02T10:00:01Z' },
            blocks: [],
            finish_reason: 'command',
          },
          conversation_updated_at: '2026-09-02T10:00:01Z',
          rotated_to_conversation_id: rotatedTo,
          telemetry: { model: 'command', provider_name: 'platform', tool_calls_count: 0, tools_called: [], execution_time_ms: 0 },
        });
      },
    );
  }

  // `/reset` archives the thread server-side. The hook's only job is to hand
  // the new id back, because the screen — not the hook — owns navigation.
  it('answers with the conversation the turn moved the athlete to', async () => {
    answerWithRotation('conv-fresh');
    const { result } = renderHook(() => useMessages());

    let rotatedTo: string | null = null;
    await act(async () => {
      rotatedTo = await result.current.sendTurn('conv-1', '/reset');
    });

    expect(rotatedTo).toBe('conv-fresh');
    // The thread it left is no longer on screen; opening the new one reads its rows.
    expect(mockGetConversationVerdicts).not.toHaveBeenCalled();
  });

  it('answers with null for an ordinary turn that moved nobody', async () => {
    answerWithRotation(undefined);
    const { result } = renderHook(() => useMessages());

    let rotatedTo: string | null = 'unset';
    await act(async () => {
      rotatedTo = await result.current.sendTurn('conv-1', 'How was my week?');
    });

    expect(rotatedTo).toBeNull();
  });
});

describe('useMessages agent welcome', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  // carnet#735: `/agent add` binds the agent, and the turn carries the welcome
  // it posted — starters included. The thread shows it right after the
  // command's answer, without re-reading the conversation.
  it('appends the welcome a binding command carried after its answer', async () => {
    const welcome = {
      id: 'w1',
      role: 'assistant',
      content: "Hi! Dravr's Tempo Agent here.",
      finish_reason: 'agent_welcome',
      created_at: '2026-10-02T10:00:02Z',
      actions: {
        title: 'To get started, you can ask me:',
        actions: [{ label: 'Plan my tempo week', action_type: 'postback', value: 'Plan my tempo week' }],
      },
    };
    mockSendTurn.mockImplementation(
      async (
        _conversationId: string,
        _text: string,
        options: { onDone?: (turn: Record<string, unknown>) => void },
      ) => {
        options.onDone?.({
          turn_id: 't1',
          user_message: { id: 'u1', role: 'user', content: '/agent add tempo', created_at: '2026-10-02T10:00:00Z' },
          assistant: {
            message: { id: 'a1', role: 'assistant', content: 'Agent selected: Tempo Agent.', created_at: '2026-10-02T10:00:01Z' },
            blocks: [],
            finish_reason: 'command',
          },
          conversation_updated_at: '2026-10-02T10:00:02Z',
          welcome_message: welcome,
          telemetry: { model: 'command', provider_name: 'platform', tool_calls_count: 0, tools_called: [], execution_time_ms: 0 },
        });
      },
    );
    const { result } = renderHook(() => useMessages());

    await act(async () => {
      await result.current.sendTurn('conv-1', '/agent add tempo');
    });

    expect(result.current.messages.map((m) => m.id)).toEqual(['u1', 'a1', 'w1']);
    expect(result.current.messages[2].actions?.actions[0].value).toBe('Plan my tempo week');
  });
});
