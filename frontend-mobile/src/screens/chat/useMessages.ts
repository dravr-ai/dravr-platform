// ABOUTME: Hook for managing chat messages state and operations
// ABOUTME: Handles loading, sending, feedback, and message rendering logic

import React, { useState, useCallback, useRef, useEffect } from 'react';
import { type FlashListRef } from '@shopify/flash-list';
import { useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { chatApi } from '../../services/api';
import { holdIdleWhileBusy, idleSignal } from '../../services/idleSignal';
import { replySceneBlocks } from '@pierre/api-client';
import type { ClaimVerdict, ReplyBlock, ReplyNotice } from '@pierre/shared-types';
import { filterDisplayMessages, statusForProgress } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import type { Message } from '../../types';

export interface MessagesState {
  messages: Message[];
  isSending: boolean;
  error: string | null;
  messageFeedback: Record<string, 'up' | 'down' | null>;
  /** Saved thumbs-down reasons, keyed by message id. */
  messageFeedbackComment: Record<string, string>;
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
   * The verdict read is in flight.
   *
   * A chip pressed before its rows landed opens the sheet on this flag rather
   * than on an empty list, so the athlete reads "loading" and not "nothing".
   */
  verdictsLoading: boolean;
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
  /**
   * Re-read the conversation's claim verdicts.
   *
   * The rows are written right after the reply row, so a chip that landed on
   * the live turn may have no row yet; the sheet asks for them on open.
   */
  refreshVerdicts: (conversationId: string) => Promise<void>;
  /**
   * Send one turn.
   *
   * Answers with the conversation the athlete is now on when the turn moved
   * them — `/reset` archives the thread and continues on a fresh one — and
   * `null` when they stayed put, which is every other turn.
   */
  sendTurn: (
    conversationId: string,
    messageText: string
  ) => Promise<string | null>;
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
  const queryClient = useQueryClient();
  const { t } = useTranslation();
  const [messages, setMessages] = useState<Message[]>([]);
  const [isSending, setIsSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [messageFeedback, setMessageFeedback] = useState<Record<string, 'up' | 'down' | null>>({});
  const [messageFeedbackComment, setMessageFeedbackComment] = useState<Record<string, string>>({});
  const [messageBlocks, setMessageBlocks] = useState<Record<string, ReplyBlock[]>>({});
  const [verdicts, setVerdicts] = useState<ClaimVerdict[]>([]);
  const [verdictsLoading, setVerdictsLoading] = useState(false);
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

  // A turn moves the thread to the top of the conversation list and rewrites
  // its preview, its time and its unread count. The list is a React Query
  // cache the tab badge reads too, so it is re-read rather than guessed at.
  const invalidateConversationList = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
  }, [queryClient]);

  // The verdict read is a separate endpoint, and a conversation that has
  // none answers with an empty list. A failure here costs the chips and
  // nothing else, so it must not take the transcript down with it.
  const refreshVerdicts = useCallback(async (conversationId: string) => {
    setVerdictsLoading(true);
    try {
      const verdictResponse = await chatApi.getConversationVerdicts(conversationId);
      setVerdicts(verdictResponse.verdicts ?? []);
    } catch (verdictErr) {
      setVerdicts([]);
      console.error('Failed to load claim verdicts:', verdictErr);
    } finally {
      setVerdictsLoading(false);
    }
  }, []);

  const loadMessages = useCallback(async (conversationId: string) => {
    try {
      setError(null);
      const response = await chatApi.getConversationMessages(conversationId);
      const allMessages = response.messages || [];

      // Drop internal LLM plumbing rows (tool_call / tool_result) so their raw
      // <tool_call>/<tool_result> XML never renders — critical for
      // messaging-origin conversations (Telegram etc.) that carry the same
      // scaffolding rows as native chat.
      setMessages(filterDisplayMessages(allMessages));

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

      await refreshVerdicts(conversationId);

      deferredScrollToBottom(100);
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Failed to load messages';
      setError(errorMessage);
      console.error('Failed to load messages:', err);
    }
  }, [deferredScrollToBottom, refreshVerdicts]);

  const sendTurn = useCallback(async (
    conversationId: string,
    messageText: string,
  ): Promise<string | null> => {
    if (!messageText.trim() || isSending) return null;

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
    // Set from the turn envelope when the turn moved the athlete to another
    // thread, and handed back so the screen can open it.
    let rotatedTo: string | null = null;
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
          const status = statusForProgress(progress);
          if (status !== null) setProgressText(t(status.key, status.params));
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
          invalidateConversationList();
          rotatedTo = turn.rotated_to_conversation_id ?? null;
        },
        onError: sendErr => {
          setError(sendErr.message);
          invalidateConversationList();
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
    return rotatedTo;
  }, [isSending, deferredScrollToBottom, invalidateConversationList]);

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
          const status = statusForProgress(progress);
          if (status !== null) setProgressText(t(status.key, status.params));
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
          invalidateConversationList();
        },
        onError: err => {
          setError(err.message);
          invalidateConversationList();
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
  }, [messages, deferredScrollToBottom, invalidateConversationList]);

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

  // The block lists are keyed by message id, so a thread's live turns would
  // otherwise keep drawing over whatever conversation is opened next.
  const clearMessages = useCallback(() => {
    setMessages([]);
    setMessageBlocks({});
    setVerdicts([]);
    setError(null);
  }, []);

  return {
    messages,
    isSending,
    error,
    messageFeedback,
    messageFeedbackComment,
    messageBlocks,
    verdicts,
    verdictsLoading,
    quotaNotice,
    progressText,
    loadMessages,
    refreshVerdicts,
    sendTurn,
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
