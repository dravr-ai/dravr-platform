// ABOUTME: Type validation tests for Pierre Mobile app
// ABOUTME: Ensures type definitions match expected shape

import type {
  User,
  Conversation,
  Message,
  ExtendedProviderStatus,
  McpToken,
} from '../src/types';

describe('Type Definitions', () => {
  describe('User type', () => {
    it('should have required fields', () => {
      const user: User = {
        id: '123',
        email: 'test@example.com',
        is_admin: false,
        role: 'user',
        tier: 'starter',
        created_at: '2024-01-01T00:00:00Z',
      };
      expect(user.id).toBe('123');
      expect(user.email).toBe('test@example.com');
    });

    it('should accept optional display_name', () => {
      const user: User = {
        id: '123',
        email: 'test@example.com',
        display_name: 'Test User',
        is_admin: false,
        role: 'user',
        tier: 'starter',
        created_at: '2024-01-01T00:00:00Z',
      };
      expect(user.display_name).toBe('Test User');
    });
  });

  describe('Conversation type', () => {
    it('should have required fields', () => {
      const conversation: Conversation = {
        id: 'conv-123',
        title: 'Test Conversation',
        model: 'gpt-4',
        total_tokens: 100,
        message_count: 5,
        created_at: '2024-01-01T00:00:00Z',
        updated_at: '2024-01-01T00:00:00Z',
      };
      expect(conversation.id).toBe('conv-123');
      expect(conversation.title).toBe('Test Conversation');
    });
  });

  describe('Message type', () => {
    it('should accept user role', () => {
      const message: Message = {
        id: 'msg-123',
        role: 'user',
        content: 'Hello',
        created_at: '2024-01-01T00:00:00Z',
      };
      expect(message.role).toBe('user');
    });

    it('should accept assistant role', () => {
      const message: Message = {
        id: 'msg-123',
        role: 'assistant',
        content: 'Hello',
        created_at: '2024-01-01T00:00:00Z',
      };
      expect(message.role).toBe('assistant');
    });
  });

  describe('ExtendedProviderStatus type', () => {
    it('names the backend behind a connected card', () => {
      const status: ExtendedProviderStatus = {
        provider: 'sciotte',
        display_name: 'Strava',
        requires_oauth: false,
        connected: true,
        connected_backend: 'strava',
        needs_reauth: false,
        capabilities: ['activities'],
        consent_required: false,
      };
      expect(status.connected_backend).toBe('strava');
    });
  });
});
