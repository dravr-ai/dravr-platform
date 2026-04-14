// ABOUTME: Unit tests for useCoachSelection hook
// ABOUTME: Tests coach auto-execute flow including message and sending state management

import { renderHook, act, waitFor } from '@testing-library/react-native';

// Mock API service
const mockListCoaches = jest.fn();
const mockRecordUsage = jest.fn();
const mockSendMessage = jest.fn();

jest.mock('../src/services/api', () => ({
  coachesApi: {
    list: (...args: unknown[]) => mockListCoaches(...args),
    recordUsage: (...args: unknown[]) => mockRecordUsage(...args),
  },
  chatApi: {
    sendMessage: (...args: unknown[]) => mockSendMessage(...args),
  },
}));

// Helper to create an Axios-shaped error for quota testing
function createAxiosQuotaError() {
  const { AxiosError, AxiosHeaders } = jest.requireActual('axios');
  const err = new AxiosError('Request failed');
  err.response = {
    status: 429,
    statusText: 'Too Many Requests',
    headers: {},
    config: { headers: new AxiosHeaders() },
    data: {
      code: 'QuotaExceeded',
      message: 'max_active_conversations quota exceeded: 2/2',
      details: {
        limit_type: 'max_active_conversations',
        current: 2,
        limit: 2,
      },
    },
  };
  return err;
}

import { useCoachSelection } from '../src/screens/chat/useCoachSelection';
import type { Coach, Message, Conversation } from '../src/types';

const createMockCoach = (overrides: Partial<Coach> = {}): Coach => ({
  id: 'coach-1',
  title: 'Marathon Coach',
  description: 'Analyze your running data and provide training recommendations',
  system_prompt: 'You are an expert marathon coach',
  startup_query: 'Analyze my recent training and provide recommendations',
  category: 'training',
  tags: ['running'],
  is_favorite: false,
  is_system: true,
  is_hidden: false,
  token_count: 500,
  use_count: 10,
  created_at: '2024-01-01T00:00:00Z',
  updated_at: '2024-01-01T00:00:00Z',
  last_used_at: null,
  ...overrides,
});

const createMockConversation = (overrides: Partial<Conversation> = {}): Conversation => ({
  id: 'conv-1',
  title: 'Chat with Marathon Coach',
  created_at: '2024-01-01T00:00:00Z',
  updated_at: '2024-01-01T00:00:00Z',
  message_count: 0,
  ...overrides,
});

describe('useCoachSelection', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockListCoaches.mockResolvedValue({ coaches: [] });
    mockRecordUsage.mockResolvedValue(undefined);
  });

  describe('loadCoaches', () => {
    it('should load and sort coaches by favorites then usage', async () => {
      const coaches = [
        createMockCoach({ id: '1', title: 'Low Usage', is_favorite: false, use_count: 5 }),
        createMockCoach({ id: '2', title: 'High Usage', is_favorite: false, use_count: 20 }),
        createMockCoach({ id: '3', title: 'Favorite', is_favorite: true, use_count: 1 }),
      ];
      mockListCoaches.mockResolvedValue({ coaches });

      const { result } = renderHook(() => useCoachSelection());

      await act(async () => {
        await result.current.loadCoaches();
      });

      expect(result.current.coaches).toHaveLength(3);
      expect(result.current.coaches[0].title).toBe('Favorite');
      expect(result.current.coaches[1].title).toBe('High Usage');
      expect(result.current.coaches[2].title).toBe('Low Usage');
    });
  });

  describe('startCoachConversation', () => {
    it('should set isSending to true at start and false at end', async () => {
      const coach = createMockCoach();
      const conversation = createMockConversation();
      const createConversation = jest.fn().mockResolvedValue(conversation);
      const setMessages = jest.fn();
      const setIsSending = jest.fn();
      const scrollToBottom = jest.fn();

      mockSendMessage.mockResolvedValue({
        user_message: { id: 'msg-1', role: 'user', content: coach.description, created_at: new Date().toISOString() },
        assistant_message: { id: 'msg-2', role: 'assistant', content: 'Here is your analysis', created_at: new Date().toISOString() },
        model: 'gemini-2.0-flash',
        execution_time_ms: 5000,
      });

      const { result } = renderHook(() => useCoachSelection());

      await act(async () => {
        await result.current.startCoachConversation(coach, {
          createConversation,
          setMessages,
          setIsSending,
          scrollToBottom,
        });
      });

      // setIsSending should be called with true first, then false
      expect(setIsSending).toHaveBeenCalledTimes(2);
      expect(setIsSending).toHaveBeenNthCalledWith(1, true);
      expect(setIsSending).toHaveBeenNthCalledWith(2, false);
    });

    it('should create conversation attached to the coach by id', async () => {
      const coach = createMockCoach({ id: 'coach-recovery', title: 'Recovery Coach' });
      const conversation = createMockConversation();
      const createConversation = jest.fn().mockResolvedValue(conversation);
      const setMessages = jest.fn();
      const setIsSending = jest.fn();
      const scrollToBottom = jest.fn();

      mockSendMessage.mockResolvedValue({
        user_message: { id: 'msg-1', role: 'user', content: coach.description, created_at: new Date().toISOString() },
        assistant_message: { id: 'msg-2', role: 'assistant', content: 'Recovery plan', created_at: new Date().toISOString() },
      });

      const { result } = renderHook(() => useCoachSelection());

      await act(async () => {
        await result.current.startCoachConversation(coach, {
          createConversation,
          setMessages,
          setIsSending,
          scrollToBottom,
        });
      });

      expect(createConversation).toHaveBeenCalledWith({
        title: 'Chat with Recovery Coach',
        coach_id: 'coach-recovery',
      });
    });

    it('should set initial user message then update with API response', async () => {
      const coach = createMockCoach({ startup_query: 'Analyze my running' });
      const conversation = createMockConversation();
      const createConversation = jest.fn().mockResolvedValue(conversation);
      const setMessages = jest.fn();
      const setIsSending = jest.fn();
      const scrollToBottom = jest.fn();

      const apiResponse = {
        user_message: { id: 'user-1', role: 'user', content: 'Analyze my running', created_at: '2024-01-01T00:00:00Z' },
        assistant_message: { id: 'asst-1', role: 'assistant', content: 'Your training looks great!', created_at: '2024-01-01T00:00:01Z' },
        model: 'gemini-2.0-flash',
        execution_time_ms: 3000,
      };
      mockSendMessage.mockResolvedValue(apiResponse);

      const { result } = renderHook(() => useCoachSelection());

      await act(async () => {
        await result.current.startCoachConversation(coach, {
          createConversation,
          setMessages,
          setIsSending,
          scrollToBottom,
        });
      });

      // First call: set initial user message (direct value, not function updater)
      expect(setMessages).toHaveBeenCalledTimes(2);

      const firstCall = setMessages.mock.calls[0][0];
      expect(Array.isArray(firstCall)).toBe(true);
      expect(firstCall).toHaveLength(1);
      expect(firstCall[0].role).toBe('user');
      expect(firstCall[0].content).toBe('Analyze my running');

      // Second call: function updater to replace temp message with API response
      const secondCall = setMessages.mock.calls[1][0];
      expect(typeof secondCall).toBe('function');

      // Simulate calling the updater with the current messages (the temp user message)
      const tempUserMsg = firstCall[0];
      const updatedMessages = secondCall([tempUserMsg]);

      // Should contain the real user message and assistant response
      expect(updatedMessages).toHaveLength(2);
      expect(updatedMessages[0].id).toBe('user-1');
      expect(updatedMessages[1].id).toBe('asst-1');
      expect(updatedMessages[1].content).toBe('Your training looks great!');
      expect(updatedMessages[1].model).toBe('gemini-2.0-flash');
      expect(updatedMessages[1].execution_time_ms).toBe(3000);
    });

    it('should send coach startup_query as the initial message', async () => {
      const coach = createMockCoach({ startup_query: 'Provide a marathon training plan' });
      const conversation = createMockConversation({ id: 'conv-42' });
      const createConversation = jest.fn().mockResolvedValue(conversation);
      const setMessages = jest.fn();
      const setIsSending = jest.fn();
      const scrollToBottom = jest.fn();

      mockSendMessage.mockResolvedValue({
        user_message: { id: 'msg-1', role: 'user', content: coach.startup_query, created_at: new Date().toISOString() },
        assistant_message: { id: 'msg-2', role: 'assistant', content: 'Plan ready', created_at: new Date().toISOString() },
      });

      const { result } = renderHook(() => useCoachSelection());

      await act(async () => {
        await result.current.startCoachConversation(coach, {
          createConversation,
          setMessages,
          setIsSending,
          scrollToBottom,
        });
      });

      expect(mockSendMessage).toHaveBeenCalledWith('conv-42', 'Provide a marathon training plan');
    });

    it('should record coach usage', async () => {
      const coach = createMockCoach({ id: 'coach-99' });
      const conversation = createMockConversation();
      const createConversation = jest.fn().mockResolvedValue(conversation);
      const setMessages = jest.fn();
      const setIsSending = jest.fn();
      const scrollToBottom = jest.fn();

      mockSendMessage.mockResolvedValue({
        user_message: { id: 'msg-1', role: 'user', content: coach.description, created_at: new Date().toISOString() },
        assistant_message: { id: 'msg-2', role: 'assistant', content: 'Done', created_at: new Date().toISOString() },
      });

      const { result } = renderHook(() => useCoachSelection());

      await act(async () => {
        await result.current.startCoachConversation(coach, {
          createConversation,
          setMessages,
          setIsSending,
          scrollToBottom,
        });
      });

      expect(mockRecordUsage).toHaveBeenCalledWith('coach-99');
    });

    it('should call scrollToBottom after receiving response', async () => {
      const coach = createMockCoach();
      const conversation = createMockConversation();
      const createConversation = jest.fn().mockResolvedValue(conversation);
      const setMessages = jest.fn();
      const setIsSending = jest.fn();
      const scrollToBottom = jest.fn();

      mockSendMessage.mockResolvedValue({
        user_message: { id: 'msg-1', role: 'user', content: coach.description, created_at: new Date().toISOString() },
        assistant_message: { id: 'msg-2', role: 'assistant', content: 'Response', created_at: new Date().toISOString() },
      });

      const { result } = renderHook(() => useCoachSelection());

      await act(async () => {
        await result.current.startCoachConversation(coach, {
          createConversation,
          setMessages,
          setIsSending,
          scrollToBottom,
        });
      });

      expect(scrollToBottom).toHaveBeenCalled();
    });

    it('should handle conversation creation failure', async () => {
      const coach = createMockCoach();
      const createConversation = jest.fn().mockRejectedValue(new Error('Failed to create conversation'));
      const setMessages = jest.fn();
      const setIsSending = jest.fn();
      const scrollToBottom = jest.fn();

      const { result } = renderHook(() => useCoachSelection());

      await act(async () => {
        await result.current.startCoachConversation(coach, {
          createConversation,
          setMessages,
          setIsSending,
          scrollToBottom,
        });
      });

      // Should still call setIsSending(false) in finally block
      expect(setIsSending).toHaveBeenLastCalledWith(false);
      // Should set error state and show alert
      expect(result.current.error).toBe('Failed to create conversation');
      // Should NOT call sendMessage
      expect(mockSendMessage).not.toHaveBeenCalled();
    });

    it('should handle API error during message send', async () => {
      const coach = createMockCoach();
      const conversation = createMockConversation();
      const createConversation = jest.fn().mockResolvedValue(conversation);
      const setMessages = jest.fn();
      const setIsSending = jest.fn();
      const scrollToBottom = jest.fn();

      mockSendMessage.mockRejectedValue(new Error('Network error'));

      const { result } = renderHook(() => useCoachSelection());

      await act(async () => {
        await result.current.startCoachConversation(coach, {
          createConversation,
          setMessages,
          setIsSending,
          scrollToBottom,
        });
      });

      // Should still reset sending state
      expect(setIsSending).toHaveBeenLastCalledWith(false);
      // extractErrorMessage returns the Error.message for standard errors
      expect(result.current.error).toBe('Network error');
    });

    it('should show quota-aware message on 429 error', async () => {
      const coach = createMockCoach();
      const conversation = createMockConversation();
      const createConversation = jest.fn().mockResolvedValue(conversation);
      const setMessages = jest.fn();
      const setIsSending = jest.fn();
      const scrollToBottom = jest.fn();

      mockSendMessage.mockRejectedValue(createAxiosQuotaError());

      const { result } = renderHook(() => useCoachSelection());

      await act(async () => {
        await result.current.startCoachConversation(coach, {
          createConversation,
          setMessages,
          setIsSending,
          scrollToBottom,
        });
      });

      expect(setIsSending).toHaveBeenLastCalledWith(false);
      expect(result.current.error).toBe(
        'Conversation limit reached (2/2). Delete an existing conversation to start a new one.'
      );
    });

    it('should use fallback message when coach has no startup_query', async () => {
      const coach = createMockCoach({ title: 'Nutrition Coach', startup_query: undefined });
      const conversation = createMockConversation();
      const createConversation = jest.fn().mockResolvedValue(conversation);
      const setMessages = jest.fn();
      const setIsSending = jest.fn();
      const scrollToBottom = jest.fn();

      mockSendMessage.mockResolvedValue({
        user_message: { id: 'msg-1', role: 'user', content: "Let's get started with Nutrition Coach!", created_at: new Date().toISOString() },
        assistant_message: { id: 'msg-2', role: 'assistant', content: 'Nutrition analysis', created_at: new Date().toISOString() },
      });

      const { result } = renderHook(() => useCoachSelection());

      await act(async () => {
        await result.current.startCoachConversation(coach, {
          createConversation,
          setMessages,
          setIsSending,
          scrollToBottom,
        });
      });

      expect(mockSendMessage).toHaveBeenCalledWith(
        expect.any(String),
        "Let's get started with Nutrition Coach!"
      );
    });
  });

  describe('handleCoachSelect', () => {
    it('should start conversation directly when provider is connected', async () => {
      const coach = createMockCoach();
      const startCoachConversation = jest.fn().mockResolvedValue(undefined);

      const { result } = renderHook(() => useCoachSelection());

      await act(async () => {
        await result.current.handleCoachSelect(coach, {
          isSending: false,
          hasConnectedProvider: () => true,
          selectedProvider: null,
          connectedProviders: [{ provider: 'strava', connected: true }],
          setSelectedProvider: jest.fn(),
          setProviderModalVisible: jest.fn(),
          startCoachConversation,
        });
      });

      expect(startCoachConversation).toHaveBeenCalledWith(coach);
    });

    it('should show provider modal when no provider is connected', async () => {
      const coach = createMockCoach();
      const setProviderModalVisible = jest.fn();

      const { result } = renderHook(() => useCoachSelection());

      await act(async () => {
        await result.current.handleCoachSelect(coach, {
          isSending: false,
          hasConnectedProvider: () => false,
          selectedProvider: null,
          connectedProviders: [],
          setSelectedProvider: jest.fn(),
          setProviderModalVisible,
          startCoachConversation: jest.fn(),
        });
      });

      expect(setProviderModalVisible).toHaveBeenCalledWith(true);
      expect(result.current.pendingCoachAction).toEqual({ coach });
    });

    it('should not start conversation while already sending', async () => {
      const coach = createMockCoach();
      const startCoachConversation = jest.fn();

      const { result } = renderHook(() => useCoachSelection());

      await act(async () => {
        await result.current.handleCoachSelect(coach, {
          isSending: true,
          hasConnectedProvider: () => true,
          selectedProvider: 'strava',
          connectedProviders: [{ provider: 'strava', connected: true }],
          setSelectedProvider: jest.fn(),
          setProviderModalVisible: jest.fn(),
          startCoachConversation,
        });
      });

      expect(startCoachConversation).not.toHaveBeenCalled();
    });

    it('should use cached provider when still connected', async () => {
      const coach = createMockCoach();
      const startCoachConversation = jest.fn().mockResolvedValue(undefined);
      const setSelectedProvider = jest.fn();

      const { result } = renderHook(() => useCoachSelection());

      await act(async () => {
        await result.current.handleCoachSelect(coach, {
          isSending: false,
          hasConnectedProvider: () => true,
          selectedProvider: 'strava',
          connectedProviders: [{ provider: 'strava', connected: true }],
          setSelectedProvider,
          setProviderModalVisible: jest.fn(),
          startCoachConversation,
        });
      });

      // Should use cached provider directly, no need to set a new one
      expect(startCoachConversation).toHaveBeenCalledWith(coach);
    });
  });
});
