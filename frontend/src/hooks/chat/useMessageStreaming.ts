// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Custom hook for handling message sending and streaming in chat
// ABOUTME: Manages streaming state, error handling, OAuth URL detection, and message metadata

import { useState, useCallback } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '../../constants/queryKeys';

interface Message {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  token_count?: number;
  created_at: string;
}

interface MessageMetadata {
  model: string;
  executionTimeMs: number;
}

/**
 * Interactive action button attached to the last assistant turn when the
 * server returned a slash-command response (e.g. `/coach` → per-coach
 * select buttons). Only live for the turn that produced them; reloading
 * conversation history shows the rendered text body without buttons.
 */
export interface MessageAction {
  label: string;
  action_type: string;
  value: string;
}

interface OAuthProvider {
  provider: string;
  connected: boolean;
  last_sync: string | null;
}

interface UseMessageStreamingOptions {
  selectedConversation: string | null;
  connectingProvider: string | null;
  oauthNotification: { provider: string; timestamp: number } | null;
  hasConnectedProvider: boolean;
  messagesData: { messages: Message[] } | undefined;
  oauthStatus: { providers: OAuthProvider[] } | undefined;
  setOauthNotification: (notification: { provider: string; timestamp: number } | null) => void;
  setConnectingProvider: (provider: string | null) => void;
}

interface UseMessageStreamingReturn {
  // State
  newMessage: string;
  isStreaming: boolean;
  streamingContent: string;
  errorMessage: string | null;
  errorCountdown: number | null;
  messageMetadata: Map<string, MessageMetadata>;
  /**
   * Action buttons attached to an assistant message, keyed by message id.
   * Populated when the server returns a slash-command response with a
   * card shape. Cleared on conversation switch.
   */
  messageActions: Map<string, MessageAction[]>;

  // Setters
  setNewMessage: React.Dispatch<React.SetStateAction<string>>;
  setErrorMessage: React.Dispatch<React.SetStateAction<string | null>>;

  // Handlers
  handleSendMessage: () => Promise<void>;
  handleKeyDown: (e: React.KeyboardEvent<HTMLTextAreaElement>) => void;
  /**
   * Submit an action's postback value as the next user message. The server
   * treats it exactly like a typed command, so `/coach select <uuid>`
   * from a button click flows through the same dispatcher.
   */
  handleActionClick: (action: MessageAction) => Promise<void>;
}

// List of trusted OAuth provider domains
const TRUSTED_OAUTH_DOMAINS = ['strava.com', 'fitbit.com', 'garmin.com', 'whoop.com', 'coros.com'];

// Check if hostname matches a trusted domain
const isTrustedDomain = (hostname: string): boolean => {
  return TRUSTED_OAUTH_DOMAINS.some(domain =>
    hostname === domain || hostname.endsWith(`.${domain}`)
  );
};

export function useMessageStreaming(options: UseMessageStreamingOptions): UseMessageStreamingReturn {
  const {
    selectedConversation,
    connectingProvider,
    oauthNotification,
    hasConnectedProvider,
    messagesData,
    oauthStatus,
    setOauthNotification,
    setConnectingProvider,
  } = options;

  const queryClient = useQueryClient();

  // State
  const [newMessage, setNewMessage] = useState('');
  const [isStreaming, setIsStreaming] = useState(false);
  const [streamingContent, setStreamingContent] = useState('');
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [errorCountdown, setErrorCountdown] = useState<number | null>(null);
  const [messageMetadata, setMessageMetadata] = useState<Map<string, MessageMetadata>>(new Map());
  const [messageActions, setMessageActions] = useState<Map<string, MessageAction[]>>(new Map());

  // Parse rate limit countdown from error message
  const parseRateLimitCountdown = useCallback((error: string | null) => {
    if (!error) {
      setErrorCountdown(null);
      return;
    }

    const match = error.match(/in (\d+) seconds/);
    if (match) {
      const seconds = parseInt(match[1], 10);
      setErrorCountdown(seconds);

      // Start countdown timer
      const timer = setInterval(() => {
        setErrorCountdown(prev => {
          if (prev === null || prev <= 1) {
            clearInterval(timer);
            setErrorMessage(null);
            return null;
          }
          return prev - 1;
        });
      }, 1000);
    }
  }, []);

  const handleSendMessage = useCallback(async () => {
    if (!newMessage.trim() || !selectedConversation || isStreaming) return;

    // Store conversation ID if connecting a provider
    if (connectingProvider) {
      sessionStorage.setItem('pierre_oauth_conversation', selectedConversation);
    }

    const displayContent = newMessage.trim();

    // Add context about connected providers
    let messageContent = displayContent;
    if (oauthNotification) {
      messageContent = `[Context: I just connected my ${oauthNotification.provider} account successfully] ${displayContent}`;
    } else if (hasConnectedProvider && (!messagesData?.messages || messagesData.messages.length === 0)) {
      const connectedProviders = oauthStatus?.providers
        ?.filter(p => p.connected)
        .map(p => p.provider.charAt(0).toUpperCase() + p.provider.slice(1))
        .join(', ');
      if (connectedProviders) {
        messageContent = `[Context: I have ${connectedProviders} connected] ${displayContent}`;
      }
    }

    setNewMessage('');
    setIsStreaming(true);
    setStreamingContent('');
    setErrorMessage(null);
    setOauthNotification(null);

    try {
      // Optimistically add user message to UI
      queryClient.setQueryData(
        QUERY_KEYS.chat.messages(selectedConversation),
        (old: { messages: Message[] } | undefined) => ({
          messages: [
            ...(old?.messages || []),
            {
              id: `temp-${Date.now()}`,
              role: 'user' as const,
              content: displayContent,
              created_at: new Date().toISOString(),
            },
          ],
        })
      );

      // Send message and stream response. X-Client-Platform tells the
      // server that a command (/coach, /group, ...) came from web chat
      // so analytics/channel_type reflect the real surface; the flag
      // also feeds PlatformCommandContext for handlers that care.
      const response = await fetch(`/api/chat/conversations/${selectedConversation}/messages`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'X-Client-Platform': 'web',
        },
        credentials: 'include',
        body: JSON.stringify({ content: messageContent }),
      });

      if (!response.ok) {
        const errorData = await response.json().catch(() => ({ message: 'Unknown error' }));
        throw new Error(errorData.message || errorData.error || 'Failed to send message');
      }

      // Parse JSON response (non-streaming with MCP tool support)
      const jsonResponse = await response.json();
      const fullContent = jsonResponse.assistant_message?.content || '';
      const assistantMessageId = jsonResponse.assistant_message?.id;
      const responseModel = jsonResponse.model;
      const responseExecutionTimeMs = jsonResponse.execution_time_ms;
      setStreamingContent(fullContent);

      // Store metadata
      if (assistantMessageId && (responseModel || responseExecutionTimeMs)) {
        setMessageMetadata(prev => {
          const updated = new Map(prev);
          updated.set(assistantMessageId!, {
            model: responseModel || 'unknown',
            executionTimeMs: responseExecutionTimeMs || 0,
          });
          return updated;
        });
      }

      // Slash-command responses carry an `actions` array (e.g. per-coach
      // select buttons on `/coach`). Attach to the assistant message id
      // so MessageItem can render clickable buttons. Actions are not
      // persisted; they live only for this turn.
      if (assistantMessageId && Array.isArray(jsonResponse.actions) && jsonResponse.actions.length > 0) {
        setMessageActions(prev => {
          const updated = new Map(prev);
          updated.set(assistantMessageId!, jsonResponse.actions as MessageAction[]);
          return updated;
        });
      }

      // Refresh messages
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.messages(selectedConversation) });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });

      // Auto-redirect to OAuth URL if connecting provider
      if (connectingProvider && fullContent) {
        const oauthUrlMatch = fullContent.match(/https?:\/\/[^\s<>[\]()]+oauth[^\s<>[\]()]*/i) ||
                             fullContent.match(/https?:\/\/[^\s<>[\]()]*strava\.com[^\s<>[\]()]*/i) ||
                             fullContent.match(/https?:\/\/[^\s<>[\]()]*fitbit\.com[^\s<>[\]()]*/i) ||
                             fullContent.match(/https?:\/\/[^\s<>[\]()]*garmin\.com[^\s<>[\]()]*/i) ||
                             fullContent.match(/https?:\/\/[^\s<>[\]()]*whoop\.com[^\s<>[\]()]*/i);

        if (oauthUrlMatch) {
          console.log(`Auto-redirecting to OAuth URL for ${connectingProvider}`);
          setTimeout(() => {
            try {
              const url = new URL(oauthUrlMatch[0]);
              if (isTrustedDomain(url.hostname) && (url.protocol === 'http:' || url.protocol === 'https:')) {
                window.location.href = url.href;
              } else {
                console.warn('OAuth redirect blocked: URL not from trusted domain');
                setConnectingProvider(null);
              }
            } catch {
              console.warn('OAuth redirect blocked: Invalid URL format');
              setConnectingProvider(null);
            }
          }, 500);
        } else {
          setConnectingProvider(null);
        }
      }
    } catch (error) {
      console.error('Failed to send message:', error);
      const errorMsg = error instanceof Error ? error.message : 'Failed to send message';
      setErrorMessage(errorMsg);
      parseRateLimitCountdown(errorMsg);
      setConnectingProvider(null);
    } finally {
      setIsStreaming(false);
      setStreamingContent('');
    }
  }, [
    newMessage,
    selectedConversation,
    isStreaming,
    queryClient,
    connectingProvider,
    oauthNotification,
    hasConnectedProvider,
    messagesData,
    oauthStatus,
    setOauthNotification,
    setConnectingProvider,
    parseRateLimitCountdown,
  ]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSendMessage();
    }
  }, [handleSendMessage]);

  /**
   * Handle a click on an interactive action button from a command
   * response. Postbacks are sent as the user's next chat message so they
   * flow through the same dispatch path as a typed command. URL actions
   * open the target in a new tab (respecting the trusted-domain check
   * we already enforce for OAuth redirects).
   */
  const handleActionClick = useCallback(async (action: MessageAction) => {
    if (action.action_type === 'url') {
      try {
        const url = new URL(action.value);
        if (isTrustedDomain(url.hostname) && (url.protocol === 'http:' || url.protocol === 'https:')) {
          window.open(url.href, '_blank', 'noopener,noreferrer');
        } else {
          console.warn('Action URL blocked: not from a trusted domain', url.hostname);
        }
      } catch (e) {
        console.warn('Action URL parse failed', e);
      }
      return;
    }
    // postback: seed the input and dispatch through the normal send path.
    setNewMessage(action.value);
    // Give React a tick to commit setNewMessage before handleSendMessage
    // reads it back; without this the send may race with a stale value.
    setTimeout(() => {
      void handleSendMessage();
    }, 0);
  }, [handleSendMessage]);

  return {
    // State
    newMessage,
    isStreaming,
    streamingContent,
    errorMessage,
    errorCountdown,
    messageMetadata,
    messageActions,

    // Setters
    setNewMessage,
    setErrorMessage,

    // Handlers
    handleSendMessage,
    handleKeyDown,
    handleActionClick,
  };
}
