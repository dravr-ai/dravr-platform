// ABOUTME: Hook for managing chat messages state and operations
// ABOUTME: Handles loading, sending, feedback, and message rendering logic

import React, { useState, useCallback, useRef, useEffect } from 'react';
import { type FlashListRef } from '@shopify/flash-list';
import { useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { chatApi } from '../../services/api';
import {
  holdIdleWhileBusy,
  idleSignal,
  trackAbsence,
  whenAthleteReturns,
} from '../../services/idleSignal';
import { replySceneBlocks, TurnIdleAbortedError, type MessagesResponse } from '@pierre/api-client';
import type { ClaimVerdict, ReplyBlock, ReplyNotice } from '@pierre/shared-types';
import { filterDisplayMessages, replyLandedSince, statusForProgress } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import type { Message } from '../../types';
import type { ChatRow } from './MessageList';

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
  flatListRef: React.RefObject<FlashListRef<ChatRow> | null>;
}

/**
 * A turn whose stream was lost while the athlete was away.
 *
 * The server finishes a turn whether or not anyone is still reading, so the
 * failure's note stands only until a read of the conversation holds the
 * reply. Every read of the thread goes through `showTranscript`, which puts
 * the note back under a transcript that does not answer the turn yet and
 * drops it once one does — so the screen's own reloads (a focus, the end of a
 * send) cannot take the note down before the reply is there.
 */
interface LostTurn {
  conversationId: string;
  /** The rows the client held when it sent the turn. */
  heldIds: ReadonlySet<string>;
  /** The question the turn sent, for a transcript that never received it. */
  question: Message;
  /** The failure's row: the note and its Retry. */
  note: Message;
  /** The failure's text, which `error` carries while the note stands. */
  failure: string;
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
  const flatListRef = useRef<FlashListRef<ChatRow>>(null);
  const scrollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // The thread these rows belong to, so a re-read scheduled for one thread
  // never paints its rows over another the athlete has opened since.
  const openConversationRef = useRef<string | null>(null);
  // The turn lost while the athlete was away whose reply has not landed yet.
  // A new turn supersedes it, so it is cleared the moment one starts.
  const lostTurnRef = useRef<LostTurn | null>(null);

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

  // Put a conversation's transcript on screen exactly as the server holds it,
  // with a lost turn's note under it for as long as the transcript does not
  // answer that turn.
  const showTranscript = useCallback(async (conversationId: string, response: MessagesResponse) => {
    // Drop internal LLM plumbing rows (tool_call / tool_result) so their raw
    // <tool_call>/<tool_result> XML never renders — critical for
    // messaging-origin conversations (Telegram etc.) that carry the same
    // scaffolding rows as native chat.
    const transcript = response.messages || [];
    const rows = filterDisplayMessages(transcript);
    const lost = lostTurnRef.current;
    if (lost?.conversationId !== conversationId) {
      setMessages(rows);
    } else if (replyLandedSince(transcript, lost.heldIds)) {
      // The reply is written: it renders from the transcript, once, and the
      // note and its failure are done.
      lostTurnRef.current = null;
      setError(null);
      setMessages(rows);
    } else {
      // Not answered yet. The note sits under the question — the server's
      // copy when it received the turn, the client's own when it never did —
      // so its Retry re-sends the right line.
      const received = transcript.some(m => m.role === 'user' && !lost.heldIds.has(m.id));
      setMessages(received ? [...rows, lost.note] : [...rows, lost.question, lost.note]);
      setError(lost.failure);
    }

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
  }, [deferredScrollToBottom, refreshVerdicts]);

  // LIMITATION(registre#440): `loadMessages` reads the caller's own conversation, so a channel group thread omits every other member's turns.
  const loadMessages = useCallback(async (conversationId: string) => {
    openConversationRef.current = conversationId;
    try {
      setError(null);
      const response = await chatApi.getConversationMessages(conversationId);
      await showTranscript(conversationId, response);
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : t('app.failedLoadMessages');
      setError(errorMessage);
      console.error('Failed to load messages:', err);
    }
  }, [showTranscript]);

  /**
   * Re-read a thread whose turn was lost while the athlete was away, now that
   * they are back.
   *
   * The server finishes a turn whether or not anyone is still reading, so the
   * reply is often already persisted by then, and the transcript shows it
   * once, in place of the note. When it is not, the note stays: it says what
   * to do, and the next read of the thread shows the reply.
   *
   * Checked again after the read lands, because the athlete may have moved on
   * while it was on the wire: a new turn supersedes the lost one, and painting
   * the transcript over it would drop that turn's question and reply.
   */
  const recoverLostReply = useCallback(async (lost: LostTurn) => {
    const stillShowing = () =>
      lostTurnRef.current === lost && openConversationRef.current === lost.conversationId;
    if (!stillShowing()) return;
    try {
      const response = await chatApi.getConversationMessages(lost.conversationId);
      if (!stillShowing()) return;
      await showTranscript(lost.conversationId, response);
    } catch (err) {
      console.error('Failed to re-read the interrupted turn:', err);
    }
  }, [showTranscript]);

  /**
   * Once a turn has settled, schedule the re-read for the athlete's return if
   * it was lost while they were away.
   *
   * After the turn, so the re-read never races the rows the turn is still
   * writing. `heldIds` names the turn: only its own lost record is re-read.
   */
  const recoverOnReturn = useCallback((heldIds: ReadonlySet<string>) => {
    const lost = lostTurnRef.current;
    if (lost?.heldIds !== heldIds) return;
    whenAthleteReturns(() => {
      void recoverLostReply(lost);
    });
  }, [recoverLostReply]);

  /**
   * What the athlete reads for a failed turn. The idle stop's error carries no
   * athlete-facing words, so its note comes from the shared catalogue, in the
   * athlete's language; every other failure already arrives worded.
   */
  const turnFailureText = useCallback(
    (failure: Error): string =>
      failure instanceof TurnIdleAbortedError ? t('chat.turnIdleAborted') : failure.message,
    [t],
  );

  /**
   * The row a failed turn leaves in the thread.
   *
   * A turn the idle stop dropped carries its own guidance — the reply may
   * still have been written — so it is not told to try again on top of that.
   */
  const failedTurnRow = useCallback((failure: Error, aborted: boolean): Message => ({
    id: `error-${Date.now()}`,
    role: 'assistant',
    content: aborted
      ? `⚠️ ${turnFailureText(failure)}`
      : `⚠️ ${turnFailureText(failure)}\n\n${t('chat.turnTryAgain')}`,
    created_at: new Date().toISOString(),
    isError: true,
  }), [t, turnFailureText]);

  const sendTurn = useCallback(async (
    conversationId: string,
    messageText: string,
  ): Promise<string | null> => {
    if (!messageText.trim() || isSending) return null;

    setIsSending(true);
    setError(null);
    openConversationRef.current = conversationId;
    lostTurnRef.current = null;

    // What the thread held before this turn, so a re-read after a lost stream
    // can tell this turn's reply from the rows that were already there.
    const heldIds: ReadonlySet<string> = new Set(messages.map(m => m.id));
    const leftDuringTurn = trackAbsence();

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
    const signal = idleSignal();
    try {
      await chatApi.sendTurn(conversationId, messageText, {
        // A turn left streaming into an app nobody has touched for the idle
        // threshold holds a server instance open; the idle watch aborts it,
        // and the thread is re-read when the athlete returns.
        signal,
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
          setError(turnFailureText(sendErr));
          invalidateConversationList();
          const errorResponse = failedTurnRow(sendErr, signal.aborted);
          const question: Message = { ...userMessage, id: `user-${Date.now()}` };
          // Failed while the athlete was away: every read of the thread keeps
          // the note until the reply has landed.
          if (leftDuringTurn()) {
            lostTurnRef.current = {
              conversationId,
              heldIds,
              question,
              note: errorResponse,
              failure: turnFailureText(sendErr),
            };
          }
          setMessages(prev => [
            ...prev.map(m => (m.id === userMessage.id ? question : m)),
            errorResponse,
          ]);
        },
      });
    } finally {
      releaseIdleHold();
    }

    deferredScrollToBottom(200);
    setIsSending(false);
    setProgressText(null);
    recoverOnReturn(heldIds);
    return rotatedTo;
  }, [isSending, messages, deferredScrollToBottom, invalidateConversationList, failedTurnRow, recoverOnReturn]);

  const retryMessage = useCallback(async (messageId: string, conversationId: string) => {
    const messageIndex = messages.findIndex(m => m.id === messageId);
    if (messageIndex <= 0) return;

    const userMessage = messages[messageIndex - 1];
    if (userMessage.role !== 'user') return;

    setMessages(prev => prev.filter(m => m.id !== messageId));
    setIsSending(true);
    setError(null);
    openConversationRef.current = conversationId;
    lostTurnRef.current = null;

    setProgressText(null);

    // Every row the client held, the retried one included: a stored reply
    // being regenerated is still on the server, where it answers the original
    // question and not the one re-sent here.
    const heldIds: ReadonlySet<string> = new Set(messages.map(m => m.id));
    const leftDuringTurn = trackAbsence();
    const retriedBlocks: ReplyBlock[] = [];
    // A streaming turn holds the client active: the athlete asked and is
    // waiting, even with the screen untouched. Released in the finally so
    // the idle threshold measures the quiet after the turn, not during it.
    const releaseIdleHold = holdIdleWhileBusy();
    const signal = idleSignal();
    try {
      await chatApi.sendTurn(conversationId, userMessage.content, {
        signal,
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
          setError(turnFailureText(err));
          invalidateConversationList();
          const errorRow = failedTurnRow(err, signal.aborted);
          if (leftDuringTurn()) {
            lostTurnRef.current = {
              conversationId,
              heldIds,
              // The re-sent line, as a row of its own: the original stays
              // where it was in the transcript.
              question: { ...userMessage, id: `user-${Date.now()}` },
              note: errorRow,
              failure: turnFailureText(err),
            };
          }
          setMessages(prev => [...prev, errorRow]);
        },
      });
    } finally {
      releaseIdleHold();
    }

    deferredScrollToBottom(200);
    setIsSending(false);
    setProgressText(null);
    recoverOnReturn(heldIds);
  }, [messages, deferredScrollToBottom, invalidateConversationList, failedTurnRow, recoverOnReturn]);

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
        setError(err instanceof Error ? err.message : t('chat.feedbackSaveFailed'));
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
        setError(err instanceof Error ? err.message : t('chat.feedbackSaveFailed'));
      }
    },
    []
  );

  // The block lists are keyed by message id, so a thread's live turns would
  // otherwise keep drawing over whatever conversation is opened next.
  const clearMessages = useCallback(() => {
    openConversationRef.current = null;
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
