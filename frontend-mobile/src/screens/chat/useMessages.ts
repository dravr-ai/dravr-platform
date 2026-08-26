// ABOUTME: Hook for managing chat messages state and operations
// ABOUTME: Handles loading, sending, insights, feedback, and message rendering logic

import React, { useState, useCallback, useRef, useEffect } from 'react';
import { type FlashListRef } from '@shopify/flash-list';
import { chatApi } from '../../services/api';
import { holdIdleWhileBusy, idleSignal } from '../../services/idleSignal';
import { replySceneBlocks } from '@pierre/api-client';
import type { ClaimVerdict, ReplyBlock, ReplyNotice } from '@pierre/shared-types';
import {
  isInsightPrompt,
  detectInsightMessages,
  createInsightPrompt,
  filterDisplayMessages,
  statusTextForProgress,
} from '@pierre/chat-utils';
import type { Message } from '../../types';

export interface MessagesState {
  messages: Message[];
  isSending: boolean;
  error: string | null;
  messageFeedback: Record<string, 'up' | 'down' | null>;
  /** Saved thumbs-down reasons, keyed by message id. */
  messageFeedbackComment: Record<string, string>;
  insightMessages: Set<string>;
  /**
   * What the server decided this turn draws, keyed by assistant message id.
   *
   * The pipeline read the mobile surface's render capabilities and produced
   * this list — the activity panel, the controls, the chips, the reconnect
   * call to action. Not persisted: a reloaded conversation has no block list
   * on the wire, so the renderer decodes its rows back into the same shape.
   */
  messageBlocks: Record<string, ReplyBlock[]>;
  /**
   * Claim verdicts attached to the loaded conversation's messages.
   *
   * The richer half of the same facts the turn's `verdicts` block carries:
   * these rows come from the conversation's verdict read, so a chip on a
   * message read back from history knows what was flagged too.
   */
  verdicts: ClaimVerdict[];
  /**
   * The quota notice the turn's own pre-turn check reported, or `null`.
   *
   * Carries the counter, its cap and its reset instant, so the banner states
   * what was actually measured instead of a countdown scraped out of prose.
   */
  quotaNotice: ReplyNotice | null;
  /**
   * What the in-flight turn is doing right now (e.g. "reading your
   * question…"), or `null` between turns.
   *
   * Read off the turn's own response body — the same one the reply arrives
   * on — so there is no second subscription to open and nothing to correlate.
   * Reset to `null` once the turn lands.
   */
  progressText: string | null;
}

export interface MessagesActions {
  loadMessages: (conversationId: string) => Promise<void>;
  sendTurn: (
    conversationId: string,
    messageText: string,
    onConversationNeeded?: () => Promise<string | null>
  ) => Promise<void>;
  createInsight: (
    content: string,
    conversationId: string | undefined,
    onConversationNeeded?: () => Promise<string | null>
  ) => Promise<void>;
  retryMessage: (messageId: string, conversationId: string) => Promise<void>;
  handleThumbsUp: (messageId: string, conversationId: string) => Promise<void>;
  handleThumbsDown: (messageId: string, conversationId: string) => Promise<void>;
  submitFeedbackReason: (
    messageId: string,
    conversationId: string,
    comment: string
  ) => Promise<void>;
  clearMessages: () => void;
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>;
  setMessageBlocks: React.Dispatch<React.SetStateAction<Record<string, ReplyBlock[]>>>;
  setIsSending: (sending: boolean) => void;
  scrollToBottom: () => void;
  flatListRef: React.RefObject<FlashListRef<Message> | null>;
}

export function useMessages(): MessagesState & MessagesActions {
  const [messages, setMessages] = useState<Message[]>([]);
  const [isSending, setIsSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [messageFeedback, setMessageFeedback] = useState<Record<string, 'up' | 'down' | null>>({});
  const [messageFeedbackComment, setMessageFeedbackComment] = useState<Record<string, string>>({});
  const [insightMessages, setInsightMessages] = useState<Set<string>>(new Set());
  const [messageBlocks, setMessageBlocks] = useState<Record<string, ReplyBlock[]>>({});
  const [verdicts, setVerdicts] = useState<ClaimVerdict[]>([]);
  const [quotaNotice, setQuotaNotice] = useState<ReplyNotice | null>(null);
  const [progressText, setProgressText] = useState<string | null>(null);
  const flatListRef = useRef<FlashListRef<Message>>(null);
  const scrollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const scrollToBottom = useCallback(() => {
    if (flatListRef.current && messages.length > 0) {
      flatListRef.current.scrollToEnd({ animated: true });
    }
  }, [messages.length]);

  // Safe deferred scroll that auto-clears previous timer
  const deferredScrollToBottom = useCallback((delayMs: number) => {
    if (scrollTimerRef.current) {
      clearTimeout(scrollTimerRef.current);
    }
    scrollTimerRef.current = setTimeout(() => {
      scrollToBottom();
      scrollTimerRef.current = null;
    }, delayMs);
  }, [scrollToBottom]);

  // Cleanup scroll timer on unmount
  useEffect(() => {
    return () => {
      if (scrollTimerRef.current) {
        clearTimeout(scrollTimerRef.current);
      }
    };
  }, []);

  const loadMessages = useCallback(async (conversationId: string) => {
    try {
      setError(null);
      const response = await chatApi.getConversationMessages(conversationId);
      const allMessages = response.messages || [];

      const detectedInsights = detectInsightMessages(allMessages);
      if (detectedInsights.size > 0) {
        setInsightMessages(prev => {
          const merged = new Set(prev);
          detectedInsights.forEach(id => merged.add(id));
          return merged;
        });
      }

      // Drop internal LLM plumbing rows (tool_call / tool_result) so their raw
      // <tool_call>/<tool_result> XML never renders — critical for
      // messaging-origin conversations (Telegram etc.) that carry the same
      // scaffolding rows as native chat. Then drop insight prompt user turns.
      const filteredMessages = filterDisplayMessages(allMessages).filter(
        (msg: Message) => !(msg.role === 'user' && isInsightPrompt(msg.content))
      );
      setMessages(filteredMessages);

      // Hydrate thumbs up/down state (and any saved reason) from the server so
      // feedback survives reloads and conversation switches.
      const ratings: Record<string, 'up' | 'down' | null> = {};
      const comments: Record<string, string> = {};
      for (const f of response.feedback ?? []) {
        ratings[f.message_id] = f.rating;
        if (f.comment) comments[f.message_id] = f.comment;
      }
      setMessageFeedback(ratings);
      setMessageFeedbackComment(comments);

      // The verdict read is a separate endpoint, and a conversation that has
      // none answers with an empty list. A failure here costs the chips and
      // nothing else, so it must not take the transcript down with it.
      try {
        const verdictResponse = await chatApi.getConversationVerdicts(conversationId);
        setVerdicts(verdictResponse.verdicts ?? []);
      } catch (verdictErr) {
        setVerdicts([]);
        console.error('Failed to load claim verdicts:', verdictErr);
      }

      deferredScrollToBottom(100);
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Failed to load messages';
      setError(errorMessage);
      console.error('Failed to load messages:', err);
    }
  }, [deferredScrollToBottom]);

  const sendTurn = useCallback(async (
    conversationId: string,
    messageText: string,
  ) => {
    if (!messageText.trim() || isSending) return;

    setIsSending(true);
    setError(null);

    const userMessage: Message = {
      id: `temp-${Date.now()}`,
      role: 'user',
      content: messageText,
      created_at: new Date().toISOString(),
    };
    setMessages(prev => [...prev, userMessage]);
    deferredScrollToBottom(200);

    setProgressText(null);

    // The reply arrives already decomposed: the server read this surface's
    // render capabilities and decided which pieces get their own block. Held
    // until `onDone` supplies the assistant message id they are keyed by.
    const turnBlocks: ReplyBlock[] = [];
    // A streaming turn holds the client active: the athlete asked and is
    // waiting, even with the screen untouched. Released in the finally so
    // the idle threshold measures the quiet after the turn, not during it.
    const releaseIdleHold = holdIdleWhileBusy();
    try {
      await chatApi.sendTurn(conversationId, messageText, {
        // A turn left streaming into a backgrounded app holds a server instance
        // open; the idle watch aborts it and the athlete re-sends on return.
        signal: idleSignal(),
        onProgress: progress => {
          const text = statusTextForProgress(progress);
          if (text !== null) setProgressText(text);
        },
        onBlock: block => {
          // A quota notice is a fact about the turn rather than about the
          // reply, and the usage banner is where it belongs. Everything else
          // is part of the message and is drawn by the renderer's switch.
          if (block.type === 'notice') {
            setQuotaNotice(block.notice);
            return;
          }
          turnBlocks.push(block);
        },
        onDone: turn => {
          const assistantId = turn.assistant.message.id;
          if (turnBlocks.length > 0 && assistantId) {
            const blocks = [...turnBlocks];
            setMessageBlocks(prev => ({ ...prev, [assistantId]: blocks }));
          }

          setMessages(prev => {
            const filtered = prev.filter(m => m.id !== userMessage.id);
            const newMessages: Message[] = [];
            if (turn.user_message?.id) {
              newMessages.push(turn.user_message);
            }
            if (assistantId) {
              newMessages.push({
                ...turn.assistant.message,
                model: turn.telemetry.model,
                execution_time_ms: turn.telemetry.execution_time_ms,
                // The envelope carries the turn's scenes; without lifting them
                // here the athlete reads the raw viz marker until the
                // conversation is reloaded and the persisted row supplies them.
                scene_blocks: replySceneBlocks(turn),
              });
            }
            return [...filtered, ...newMessages];
          });
        },
        onError: sendErr => {
          setError(sendErr.message);
          const errorResponse: Message = {
            id: `error-${Date.now()}`,
            role: 'assistant',
            content: `⚠️ ${sendErr.message}\n\nPlease try again.`,
            created_at: new Date().toISOString(),
            isError: true,
          };
          setMessages(prev => {
            const updated = prev.map(m =>
              m.id === userMessage.id ? { ...m, id: `user-${Date.now()}` } : m
            );
            return [...updated, errorResponse];
          });
        },
      });
    } finally {
      releaseIdleHold();
    }

    deferredScrollToBottom(200);
    setIsSending(false);
    setProgressText(null);
  }, [isSending, deferredScrollToBottom]);

  const createInsight = useCallback(async (
    content: string,
    conversationId: string | undefined,
    onConversationNeeded?: () => Promise<string | null>
  ) => {
    if (isSending) return;

    let resolvedConversationId = conversationId;
    if (!resolvedConversationId && onConversationNeeded) {
      resolvedConversationId = (await onConversationNeeded()) ?? undefined;
      if (!resolvedConversationId) return;
    }
    if (!resolvedConversationId) return;

    setIsSending(true);
    setError(null);
    const insightPrompt = createInsightPrompt(content);
    deferredScrollToBottom(200);

    // An insight answers as one JSON document rather than a frame stream, so
    // no progress arrives and the strip stays hidden for the whole turn.
    // A streaming turn holds the client active: the athlete asked and is
    // waiting, even with the screen untouched. Released in the finally so
    // the idle threshold measures the quiet after the turn, not during it.
    const releaseIdleHold = holdIdleWhileBusy();
    try {
      await chatApi.sendTurn(resolvedConversationId, insightPrompt, {
        signal: idleSignal(),
        onDone: turn => {
          const insightId = turn.assistant.message.id;
          if (!insightId) return;
          setInsightMessages(prev => {
            const updated = new Set(prev);
            updated.add(insightId);
            return updated;
          });
          setMessages(prev => [...prev, {
            ...turn.assistant.message,
            model: turn.telemetry.model,
            execution_time_ms: turn.telemetry.execution_time_ms,
            scene_blocks: replySceneBlocks(turn),
          }]);
        },
        onError: err => {
          setError(err.message);
          console.error('Failed to create insight:', err);
        },
      });
    } finally {
      releaseIdleHold();
    }

    deferredScrollToBottom(200);
    setIsSending(false);
  }, [isSending, deferredScrollToBottom]);

  const retryMessage = useCallback(async (messageId: string, conversationId: string) => {
    const messageIndex = messages.findIndex(m => m.id === messageId);
    if (messageIndex <= 0) return;

    const userMessage = messages[messageIndex - 1];
    if (userMessage.role !== 'user') return;

    setMessages(prev => prev.filter(m => m.id !== messageId));
    setIsSending(true);
    setError(null);

    setProgressText(null);

    const retriedBlocks: ReplyBlock[] = [];
    // A streaming turn holds the client active: the athlete asked and is
    // waiting, even with the screen untouched. Released in the finally so
    // the idle threshold measures the quiet after the turn, not during it.
    const releaseIdleHold = holdIdleWhileBusy();
    try {
      await chatApi.sendTurn(conversationId, userMessage.content, {
        signal: idleSignal(),
        onProgress: progress => {
          const text = statusTextForProgress(progress);
          if (text !== null) setProgressText(text);
        },
        onBlock: block => {
          if (block.type === 'notice') {
            setQuotaNotice(block.notice);
            return;
          }
          retriedBlocks.push(block);
        },
        onDone: turn => {
          const retriedId = turn.assistant.message.id;
          if (!retriedId) return;
          if (retriedBlocks.length > 0) {
            const blocks = [...retriedBlocks];
            setMessageBlocks(prev => ({ ...prev, [retriedId]: blocks }));
          }
          setMessages(prev => [...prev, {
            ...turn.assistant.message,
            model: turn.telemetry.model,
            execution_time_ms: turn.telemetry.execution_time_ms,
            // The envelope carries the turn's scenes; without lifting them here
            // the athlete reads the raw viz marker until the conversation is
            // reloaded and the persisted row supplies them.
            scene_blocks: replySceneBlocks(turn),
          }]);
        },
        onError: err => {
          setError(err.message);
          setMessages(prev => [...prev, {
            id: `error-${Date.now()}`,
            role: 'assistant',
            content: `⚠️ ${err.message}\n\nPlease try again.`,
            created_at: new Date().toISOString(),
            isError: true,
          }]);
        },
      });
    } finally {
      releaseIdleHold();
    }

    deferredScrollToBottom(200);
    setIsSending(false);
    setProgressText(null);
  }, [messages, deferredScrollToBottom]);

  // Apply a rating change optimistically and persist it. Clicking the active
  // rating again toggles it off (DELETE); otherwise the rating is upserted.
  // On failure the optimistic change is reverted and an error surfaced.
  const applyFeedback = useCallback(
    async (messageId: string, conversationId: string, rating: 'up' | 'down') => {
      const previous = messageFeedback[messageId] ?? null;
      const next: 'up' | 'down' | null = previous === rating ? null : rating;

      setMessageFeedback(prev => ({ ...prev, [messageId]: next }));
      // Switching away from a down-rating drops its reason.
      if (next !== 'down') {
        setMessageFeedbackComment(prev => {
          if (!(messageId in prev)) return prev;
          const updated = { ...prev };
          delete updated[messageId];
          return updated;
        });
      }

      try {
        if (next === null) {
          await chatApi.deleteMessageFeedback(conversationId, messageId);
        } else {
          await chatApi.submitMessageFeedback(conversationId, messageId, next);
        }
      } catch (err) {
        setMessageFeedback(prev => ({ ...prev, [messageId]: previous }));
        setError(err instanceof Error ? err.message : 'Failed to save feedback');
      }
    },
    [messageFeedback]
  );

  const handleThumbsUp = useCallback(
    (messageId: string, conversationId: string) => applyFeedback(messageId, conversationId, 'up'),
    [applyFeedback]
  );

  const handleThumbsDown = useCallback(
    (messageId: string, conversationId: string) => applyFeedback(messageId, conversationId, 'down'),
    [applyFeedback]
  );

  // Persist an optional thumbs-down reason on the existing feedback row. The
  // down rating is already saved; this only adds/updates the comment.
  const submitFeedbackReason = useCallback(
    async (messageId: string, conversationId: string, comment: string) => {
      const trimmed = comment.trim();
      setMessageFeedbackComment(prev => ({ ...prev, [messageId]: trimmed }));
      try {
        await chatApi.submitMessageFeedback(
          conversationId,
          messageId,
          'down',
          trimmed || undefined
        );
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to save feedback');
      }
    },
    []
  );

  const clearMessages = useCallback(() => {
    setMessages([]);
    setVerdicts([]);
    setError(null);
  }, []);

  return {
    messages,
    isSending,
    error,
    messageFeedback,
    messageFeedbackComment,
    insightMessages,
    messageBlocks,
    verdicts,
    quotaNotice,
    progressText,
    loadMessages,
    sendTurn,
    createInsight,
    retryMessage,
    handleThumbsUp,
    handleThumbsDown,
    submitFeedbackReason,
    clearMessages,
    setMessages,
    setMessageBlocks,
    setIsSending,
    scrollToBottom,
    flatListRef,
  };
}
