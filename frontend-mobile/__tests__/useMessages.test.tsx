// ABOUTME: Unit tests for useMessages hook
// ABOUTME: Tests message state management, sending, setMessages/setIsSending exposure

import { renderHook, act } from '@testing-library/react-native';

// Mock API service
const mockGetConversationMessages = jest.fn();
const mockSendMessage = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversationMessages: (...args: unknown[]) => mockGetConversationMessages(...args),
    sendMessage: (...args: unknown[]) => mockSendMessage(...args),
  },
}));

jest.mock('@pierre/chat-utils', () => ({
  isInsightPrompt: () => false,
  detectInsightMessages: () => new Set(),
  createInsightPrompt: (content: string) => `[INSIGHT] ${content}`,
}));

import { useMessages } from '../src/screens/chat/useMessages';
import type { Message } from '../src/types';

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
        assistant_message: { id: 'asst-1', role: 'assistant', content: 'Hi there!', created_at: '2024-01-01T00:00:01Z' },
        model: 'gemini-2.0-flash',
        execution_time_ms: 2000,
      };
      mockSendMessage.mockResolvedValue(apiResponse);

      const { result } = renderHook(() => useMessages());

      await act(async () => {
        await result.current.sendMessage('conv-1', 'Hello');
      });

      expect(mockSendMessage).toHaveBeenCalledWith('conv-1', 'Hello');
      expect(result.current.messages).toHaveLength(2);
      expect(result.current.messages[0].role).toBe('user');
      expect(result.current.messages[1].role).toBe('assistant');
      expect(result.current.isSending).toBe(false);
    });

    it('should handle API error and show error message', async () => {
      mockSendMessage.mockRejectedValue(new Error('Network timeout'));

      const { result } = renderHook(() => useMessages());

      await act(async () => {
        await result.current.sendMessage('conv-1', 'Hello');
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
      mockSendMessage.mockImplementation(() => new Promise(() => {})); // Never resolves

      const { result } = renderHook(() => useMessages());

      // Start first send (won't resolve)
      act(() => {
        result.current.sendMessage('conv-1', 'First');
      });

      // Try to send again while first is pending
      await act(async () => {
        await result.current.sendMessage('conv-1', 'Second');
      });

      // Should only have been called once
      expect(mockSendMessage).toHaveBeenCalledTimes(1);
    });
  });
});
