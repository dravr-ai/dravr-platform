// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The chat surface: one open thread, its header info drawer, and the composer
// ABOUTME: Coaches and groups are commands here — no coach CRUD, no group picker, no welcome grid

import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { chatApi, providersApi } from '../services/api';
import { holdIdleWhileBusy, idleSignal } from '../services/api/idleSignal';
import { track } from '../services/analytics';
import {
  avatarSlot,
  defaultConversationTitle,
  initialsFor,
  statusForProgress,
  trustedActionUrl,
} from '@pierre/chat-utils';
import {
  MessageList,
  MessageInput,
  ChatComposeMenu,
  ChatEmptyState,
  ConversationInfoPanel,
} from './chat';
import VerdictDrawer from './chat/VerdictDrawer';
import ChatShell from './chat/ChatShell';
import ThreadHeader from './chat/ThreadHeader';
import ConversationList from './dashboard/ConversationList';
import { useIsDesktop } from '../hooks/useBreakpoint';
import UsageWarningBanner from './chat/UsageWarningBanner';
import { ConnectProviderBanner } from './ConnectProviderBanner';
import { useUsageStatus } from '../hooks/useUsageStatus';
import {
  cachedConversations,
  useConversationList,
  useConversationMutations,
} from '../hooks/useConversationList';
import { useMarkConversationRead } from '../hooks/useMarkConversationRead';
import { useCoachInfo } from '../hooks/useCoachInfo';
import { useSuccessToast, useInfoToast, useErrorToast } from './ui';
import { QUERY_KEYS } from '../constants/queryKeys';
import { replySceneBlocks } from '@pierre/api-client';
import type { ChatMessageAction, ClaimVerdict, ReplyBlock } from '@pierre/shared-types';
import type {
  Message,
  MessageMetadata,
  MessageFeedback,
  OAuthNotification,
} from './chat';
import type { MessageFeedbackEntry } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';

/**
 * The id prefix of the user row appended to the transcript while a turn is in
 * flight. It has no server id, so the read marker leaves it out of the
 * "newest persisted message" it advances to.
 */
const OPTIMISTIC_USER_ID_PREFIX = 'user-';

/** The newest row the server has persisted — the one the read marker follows. */
function latestPersistedMessageId(messages: Message[] | undefined): string | null {
  if (!messages) return null;
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (!messages[i].id.startsWith(OPTIMISTIC_USER_ID_PREFIX)) return messages[i].id;
  }
  return null;
}

/**
 * The header line naming the connected providers, agreeing in number with
 * how many there are: "Strava connected" / "Strava, Garmin connectés".
 */
function connectedProvidersLine(
  t: (key: string, values?: Record<string, string | number>) => string,
  names: string[],
): string {
  return t(names.length === 1 ? 'chat.providersConnectedOne' : 'chat.providersConnectedN', {
    providers: names.join(', '),
  });
}

/**
 * A composer action the shell hands the chat surface.
 *
 * `draft` seeds the composer and leaves the athlete to press send; `send`
 * dispatches the text as a turn on its own. Both start a conversation first
 * when none is open — an invite link opened cold has no thread to speak into.
 */
export interface PendingComposerAction {
  kind: 'draft' | 'send';
  text: string;
}

interface ChatTabProps {
  selectedConversation: string | null;
  onSelectConversation: (id: string | null) => void;
  /**
   * Dashboard route navigator, `tab[/subview]`. Editing a coach leaves for
   * `discover/<coachId>`, which is where the edit sheet lives.
   */
  onNavigate?: (route: string) => void;
  /** Text the shell wants drafted or sent in a thread — the invite deep link uses it. */
  pendingComposerAction?: PendingComposerAction | null;
  /** Called once the action above has been drafted or dispatched. */
  onPendingComposerActionConsumed?: () => void;
}

/** The stamp the server puts on a slash-command turn, both rows of it. */
const COMMAND_FINISH_REASON = 'command';

export default function ChatTab({
  selectedConversation,
  onSelectConversation,
  onNavigate,
  pendingComposerAction,
  onPendingComposerActionConsumed,
}: ChatTabProps) {
  const { t, language } = useTranslation();
  const queryClient = useQueryClient();
  const showSuccessToast = useSuccessToast();
  const showInfoToast = useInfoToast();
  const showErrorToast = useErrorToast();
  const [newMessage, setNewMessage] = useState('');
  const [isStreaming, setIsStreaming] = useState(false);
  const [streamingContent, setStreamingContent] = useState('');
  // What the in-flight turn is doing right now — the stage it entered or the
  // tool it is calling — read off the turn's own `progress` frames. Same
  // response body the reply arrives on, so there is nothing to correlate.
  const [progressStatusText, setProgressStatusText] = useState<string | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [oauthNotification, setOauthNotification] = useState<OAuthNotification | null>(null);
  const [pendingPrompt, setPendingPrompt] = useState<string | null>(null);
  const [pendingCoachId, setPendingCoachId] = useState<string | null>(null);
  const [messageMetadata, setMessageMetadata] = useState<Map<string, MessageMetadata>>(new Map());
  const [messageFeedback, setMessageFeedback] = useState<Map<string, MessageFeedback>>(new Map());
  // Saved thumbs-down reasons, keyed by message id. Hydrated from the server
  // alongside the ratings so a reload re-renders the comment.
  const [messageFeedbackComment, setMessageFeedbackComment] = useState<Map<string, string>>(new Map());
  // What the server decided this turn draws, keyed by assistant message id.
  // Not persisted: a reloaded conversation has no block list on the wire, so
  // its rows are decoded back into the same shape by the renderer.
  const [messageBlocks, setMessageBlocks] = useState<Map<string, ReplyBlock[]>>(new Map());
  // The header's info drawer — Group info, Coach info or the plain thread's
  // own controls. The "+" menu's "Add someone" opens the same drawer with the
  // participants control already expanded.
  const [infoOpen, setInfoOpen] = useState(false);
  const [infoOpensParticipants, setInfoOpensParticipants] = useState(false);

  const inputRef = useRef<HTMLTextAreaElement>(null);

  // Fetch usage quota status for warning banners
  const usageStatus = useUsageStatus();

  // Fetch provider status (includes both OAuth and non-OAuth providers like synthetic)
  const { data: providersData, isSuccess: providersLoaded } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
  });

  const hasConnectedProvider = providersData?.providers?.some(p => p.connected) ?? false;
  // Gated on the query having actually answered, not on the `?? false` default.
  // While loading, `hasConnectedProvider` is false for everyone, so keying the
  // banner off it alone flashes a connect-provider nudge at users who are
  // connected — every chat load.
  const showConnectBanner = providersLoaded && !hasConnectedProvider;

  // Fetch messages for selected conversation
  const { data: messagesData, isLoading: messagesLoading } = useQuery<{ messages: Message[]; feedback?: MessageFeedbackEntry[] }>({
    queryKey: QUERY_KEYS.chat.messages(selectedConversation),
    queryFn: () => chatApi.getConversationMessages(selectedConversation!),
    enabled: !!selectedConversation,
    // Messaging turns arrive async via inbound webhook with no websocket push
    // to the web client. Refetch when the tab/window regains focus so a reply
    // sent from Telegram (or any channel) into an open conversation appears
    // without a manual reload.
    refetchOnWindowFocus: true,
  });

  // The open thread reads itself: the marker advances once the messages
  // resolve and again on every reply that lands, so the row's unread count
  // and the nav badge clear as the athlete reads.
  const latestMessageId = useMemo(
    () => latestPersistedMessageId(messagesData?.messages),
    [messagesData],
  );
  useMarkConversationRead(selectedConversation, latestMessageId);

  // Hydrate thumbs up/down state (and any saved reason) from the server whenever
  // the messages load/refetch, so feedback survives reloads and conversation
  // switches. The server is the source of truth — this replaces local state.
  useEffect(() => {
    const feedback = messagesData?.feedback;
    if (!feedback) return;
    const ratings = new Map<string, MessageFeedback>();
    const comments = new Map<string, string>();
    for (const f of feedback) {
      ratings.set(f.message_id, f.rating);
      if (f.comment) comments.set(f.message_id, f.comment);
    }
    setMessageFeedback(ratings);
    setMessageFeedbackComment(comments);
  }, [messagesData]);

  // Claim verdicts attached to messages in the selected conversation.
  // Refetched alongside messages so a coach reply that triggers verification
  // surfaces its chip without a manual reload.
  const { data: verdictsData, isFetching: verdictsFetching, refetch: refetchVerdicts } = useQuery({
    queryKey: ['chat', 'verdicts', selectedConversation],
    queryFn: () => chatApi.getConversationVerdicts(selectedConversation!),
    enabled: !!selectedConversation,
  });
  const verdicts: ClaimVerdict[] = verdictsData?.verdicts ?? [];

  // The list is the one source for the open thread's row: its title, the
  // coach it is bound to and the group it is scoped to all come from there,
  // so the header and the info drawer read the same record the sidebar draws.
  const { conversations } = useConversationList();
  const { rename, remove } = useConversationMutations();
  const activeConversation = useMemo(
    () => conversations.find(c => c.id === selectedConversation) ?? null,
    [conversations, selectedConversation],
  );

  // `pendingCoachId` covers a freshly created conversation whose `coach_id`
  // has not yet been written back to the list.
  const { coach: activeCoach } = useCoachInfo(activeConversation?.coach_id ?? pendingCoachId);
  const activeCoachTitle = activeCoach?.title ?? null;

  // What the header names: the group, the coach, or the thread's own title.
  const headerTitle = useMemo<string>(() => {
    if (activeConversation?.group_name) return activeConversation.group_name;
    if (activeCoachTitle) return activeCoachTitle;
    return activeConversation?.title?.trim() || t('app.newConversation');
  }, [activeConversation, activeCoachTitle, t]);

  // The line under the name: the coach's handle, or what the coach can see.
  const connectedProviderNames = useMemo(
    () => (providersData?.providers ?? []).filter((p) => p.connected).map((p) => p.display_name),
    [providersData],
  );
  const providerStatusLine = useMemo<string | null>(() => {
    if (connectedProviderNames.length > 0) return connectedProvidersLine(t, connectedProviderNames);
    return providersLoaded ? t('chat.noProviderStatus') : null;
  }, [connectedProviderNames, providersLoaded, t]);
  const headerSubtitle = useMemo<string | null>(() => {
    if (activeConversation?.group_name) return t('chat.groupChatBadge');
    if (activeConversation?.coach_handle) return `@${activeConversation.coach_handle}`;
    return providerStatusLine;
  }, [activeConversation, providerStatusLine, t]);

  // Below the desktop breakpoint the list hides behind the open thread, so
  // the header carries the way back to it.
  const isDesktop = useIsDesktop();

  // The message whose verdicts the drawer shows. The rows are written right
  // after the reply row, so a chip that landed before the read did opens the
  // drawer on a refetch rather than on nothing.
  const [verdictMessageId, setVerdictMessageId] = useState<string | null>(null);
  const drawerVerdicts = useMemo(
    () => (verdictMessageId ? verdicts.filter((v) => v.message_id === verdictMessageId) : []),
    [verdicts, verdictMessageId],
  );
  const handleShowVerdict = useCallback(
    (_rows: ClaimVerdict[], messageId: string) => {
      setVerdictMessageId(messageId);
      if (!verdicts.some((v) => v.message_id === messageId)) void refetchVerdicts();
    },
    [verdicts, refetchVerdicts],
  );

  const handleAskAboutClaim = useCallback((verdict: ClaimVerdict) => {
    setNewMessage(
      t('app.backUpClaim', { claim: verdict.claim_text }),
    );
  }, [t]);

  // Mutations. Takes an optional coach ID; the server resolves the
  // coach's system prompt at runtime from the coaches table.
  const createConversation = useMutation<{ id: string }, Error, string | void>({
    mutationFn: (coachId) => {
      // Named for the moment it starts, in the viewer's language and on the
      // same 24-hour clock the list row shows; a rename replaces it.
      const defaultTitle = defaultConversationTitle(
        t('chat.newConversationTitlePrefix'),
        new Date(),
        language,
      );
      return chatApi.createConversation({
        title: defaultTitle,
        coach_id: coachId || pendingCoachId || undefined,
      });
    },
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
      onSelectConversation(data.id);
      setPendingCoachId(null);
    },
    onError: (error) => {
      setPendingCoachId(null);
      // The server caps active conversations (max_active_conversations) and
      // returns 429 QuotaExceeded once a user is at the limit. Without this the
      // coach click silently did nothing — surface a clear, actionable message.
      const apiError = error as {
        response?: { status?: number; data?: { code?: string; details?: { limit?: number } } };
      };
      const res = apiError.response;
      if (res?.status === 429 || res?.data?.code === 'QuotaExceeded') {
        const limit = res?.data?.details?.limit;
        showErrorToast(
          t('chat.conversationLimitTitle'),
          t('chat.conversationLimitBody', { limit: limit ?? '' })
        );
        return;
      }
      showErrorToast(
        t('app.couldNotStartChat'),
        t('app.conversationCreateFailed')
      );
    },
  });

  /** Stable handle on the mutation above, so callbacks can depend on it. */
  const startConversation = createConversation.mutate;

  // Restore message metadata (model label) from conversation when loading existing messages
  useEffect(() => {
    if (!selectedConversation || !messagesData?.messages?.length) return;
    const conversation = cachedConversations(queryClient).find(c => c.id === selectedConversation);
    if (!conversation?.model) return;

    setMessageMetadata(prev => {
      const newMap = new Map(prev);
      let changed = false;
      for (const msg of messagesData.messages) {
        if (msg.role === 'assistant' && !newMap.has(msg.id)) {
          newMap.set(msg.id, { model: conversation.model!, executionTimeMs: 0 });
          changed = true;
        }
      }
      return changed ? newMap : prev;
    });
  }, [selectedConversation, messagesData, queryClient]);

  // Focus input when conversation is selected; an info drawer left open
  // belongs to the previous thread, so it closes with it.
  useEffect(() => {
    setInfoOpen(false);
    setInfoOpensParticipants(false);
    if (selectedConversation) {
      inputRef.current?.focus();
    }
  }, [selectedConversation]);

  // OAuth completion listener
  useEffect(() => {
    let isProcessingOAuth = false;

    const extractOAuthData = () => {
      const stored = localStorage.getItem('pierre_oauth_result');
      if (!stored) return null;
      localStorage.removeItem('pierre_oauth_result');

      try {
        const result = JSON.parse(stored);
        const fiveMinutesAgo = Date.now() - 5 * 60 * 1000;

        if (result.type === 'oauth_completed' && result.success && result.timestamp > fiveMinutesAgo) {
          const savedConversation = sessionStorage.getItem('pierre_oauth_conversation');
          if (savedConversation) sessionStorage.removeItem('pierre_oauth_conversation');
          return { result, savedConversation };
        } else if (result.timestamp <= fiveMinutesAgo) {
          sessionStorage.removeItem('pierre_oauth_conversation');
        }
      } catch {
        // Ignore parse errors
      }
      return null;
    };

    const processOAuthData = (data: { result: { provider: string }; savedConversation: string | null }) => {
      if (isProcessingOAuth) return;
      isProcessingOAuth = true;

      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.oauth.status() });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.profile() });

      const providerDisplay = data.result.provider.charAt(0).toUpperCase() + data.result.provider.slice(1);
      setOauthNotification({ provider: providerDisplay, timestamp: Date.now() });

      if (data.savedConversation) {
        onSelectConversation(data.savedConversation);
      }

      setTimeout(() => {
        isProcessingOAuth = false;
      }, 500);
    };

    const checkAndProcessOAuthResult = () => {
      const data = extractOAuthData();
      if (data) {
        processOAuthData(data);
      }
    };

    const handleOAuthMessage = (event: MessageEvent) => {
      // Validate origin to prevent cross-origin message injection
      if (event.origin !== window.location.origin) return;
      if (event.data?.type === 'oauth_completed') {
        const { provider, success } = event.data;
        if (success && !isProcessingOAuth) {
          const savedConversation = sessionStorage.getItem('pierre_oauth_conversation');
          if (savedConversation) sessionStorage.removeItem('pierre_oauth_conversation');
          processOAuthData({ result: { provider }, savedConversation });
        }
      }
    };

    const handleStorageChange = (event: StorageEvent) => {
      if (event.key === 'pierre_oauth_result' && event.newValue) {
        const data = extractOAuthData();
        if (data) {
          processOAuthData(data);
        }
      }
    };

    const handleVisibilityChange = () => {
      if (document.visibilityState === 'visible') {
        checkAndProcessOAuthResult();
      }
    };

    const handleFocus = () => {
      checkAndProcessOAuthResult();
    };

    window.addEventListener('message', handleOAuthMessage);
    window.addEventListener('storage', handleStorageChange);
    document.addEventListener('visibilitychange', handleVisibilityChange);
    window.addEventListener('focus', handleFocus);
    checkAndProcessOAuthResult();

    return () => {
      window.removeEventListener('message', handleOAuthMessage);
      window.removeEventListener('storage', handleStorageChange);
      document.removeEventListener('visibilitychange', handleVisibilityChange);
      window.removeEventListener('focus', handleFocus);
    };
  }, [queryClient, onSelectConversation]);

  /**
   * Send one turn and fold the result into the transcript.
   *
   * Takes its content as an argument rather than reading the composer, so
   * every caller — the send button, a queued coach prompt, a command action
   * button, a retry — dispatches the same way. The composer is state; a
   * caller that had to seed it and then wait for React to commit is what
   * produced the simulated button clicks this replaced.
   */
  const sendTurn = useCallback(async (displayContent: string) => {
    if (!displayContent || !selectedConversation || isStreaming) return;

    setIsStreaming(true);
    setStreamingContent('');
    setErrorMessage(null);
    track({ name: 'feature_engaged', props: { feature: 'chat_message_sent' } });

    const userMessageId = `${OPTIMISTIC_USER_ID_PREFIX}${Date.now()}`;
    const tempUserMessage: Message = {
      id: userMessageId,
      role: 'user',
      content: displayContent,
      created_at: new Date().toISOString(),
    };

    queryClient.setQueryData(QUERY_KEYS.chat.messages(selectedConversation), (old: { messages: Message[] } | undefined) => ({
      messages: [...(old?.messages || []), tempUserMessage],
    }));

    setProgressStatusText(null);

    let assembled = '';
    // The server decided which pieces this surface draws; `onBlock` collects
    // them in that order. Held here until `onDone` supplies the assistant
    // message id they are keyed by.
    const turnBlocks: ReplyBlock[] = [];
    // A streaming turn holds the client active: the athlete asked a question
    // and is watching for the answer, even with the mouse untouched. Released
    // in the `finally` below, so the idle threshold measures the quiet AFTER
    // the turn rather than racing the model.
    const releaseIdleHold = holdIdleWhileBusy();
    await chatApi.sendTurn(selectedConversation, displayContent, {
      // A turn left streaming into a tab abandoned mid-answer still holds a
      // server instance open; the hold above covers the athlete who is
      // waiting, the signal covers the one who walked away.
      signal: idleSignal(),
      onDelta: delta => {
        assembled += delta;
        setStreamingContent(assembled);
      },
      onProgress: progress => {
        // One vocabulary, rendered the same way here and on mobile and in the
        // messaging channels' placeholder edits.
        const status = statusForProgress(progress);
        if (status !== null) setProgressStatusText(t(status.key, status.params));
      },
      onBlock: block => {
        // A quota notice is a fact about the turn rather than about the reply,
        // and the conversation banner is where it belongs. Everything else is
        // part of the message and is drawn by the renderer's block switch.
        if (block.type === 'notice') {
          usageStatus.applyNotice(block.notice);
          return;
        }
        turnBlocks.push(block);
      },
      onDone: turn => {
        const assistantMessageId = turn.assistant.message.id;
        const { model, execution_time_ms: executionTimeMs } = turn.telemetry;

        if (assistantMessageId && model) {
          setMessageMetadata(prev => {
            const newMap = new Map(prev);
            newMap.set(assistantMessageId, { model, executionTimeMs });
            return newMap;
          });
        }

        // The panel, the controls, the chips and the reconnect call to action
        // all came from the block walk above; nothing here sniffs a flat field
        // or a sentence to work out what to draw.
        if (turnBlocks.length > 0 && assistantMessageId) {
          const blocks = [...turnBlocks];
          setMessageBlocks(prev => {
            const newMap = new Map(prev);
            newMap.set(assistantMessageId, blocks);
            return newMap;
          });
        }

        // Inject the persisted assistant message into the cache directly so
        // the streaming bubble can be cleared in the same batch — invalidating
        // and waiting for the refetch leaves a window where both the streaming
        // bubble (assembled deltas) and the just-arrived persisted message
        // render together, surfacing as a duplicated opening sentence. The
        // turn envelope carries the whole turn, so the assistant message it
        // holds already includes any post-processing additions the refetch
        // would have brought in.
        const assistantMessage = turn.assistant.message;
        if (assistantMessage.id && assistantMessage.content) {
          const persisted: Message = {
            id: assistantMessage.id,
            role: assistantMessage.role ?? 'assistant',
            content: assistantMessage.content,
            token_count: assistantMessage.token_count,
            created_at: assistantMessage.created_at ?? new Date().toISOString(),
            scene_blocks: replySceneBlocks(turn),
          };
          queryClient.setQueryData(
            QUERY_KEYS.chat.messages(selectedConversation),
            (old: { messages: Message[] } | undefined) => {
              const existing = old?.messages ?? [];
              if (existing.some(m => m.id === persisted.id)) {
                return { messages: existing };
              }
              return { messages: [...existing, persisted] };
            },
          );
        }

        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });

        // A slash command is the one turn that can rewrite what the header and
        // the info panel draw — `/coach add` binds a coach, `/coach remove`
        // detaches one, `/discover install` puts a new coach on the athlete's
        // list. Both read the coach set, so a command turn refreshes it; an
        // LLM turn never changes it and is left alone.
        if (turn.assistant.finish_reason === COMMAND_FINISH_REASON) {
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.list() });
        }
      },
      onError: error => {
        setErrorMessage(error.message);
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.messages(selectedConversation) });
      },
    });

    releaseIdleHold();
    setIsStreaming(false);
    setStreamingContent('');
    // The delivered assistant reply is now the source of truth; the progress
    // line has nothing left to say.
    setProgressStatusText(null);
    usageStatus.invalidate();
  }, [selectedConversation, isStreaming, queryClient, usageStatus]);

  /** The composer's own send: hand the typed text to {@link sendTurn} and clear the box. */
  const handleSendMessage = useCallback(() => {
    const typed = newMessage.trim();
    if (!typed) return;
    setNewMessage('');
    void sendTurn(typed);
  }, [newMessage, sendTurn]);

  // A prompt queued while the conversation was still being created is sent
  // as soon as one exists. It goes straight to `sendTurn`; seeding the
  // composer and clicking its own button was the round-trip this replaced.
  useEffect(() => {
    if (pendingPrompt && selectedConversation && !isStreaming) {
      const promptToSend = pendingPrompt;
      setPendingPrompt(null);
      void sendTurn(promptToSend);
    }
  }, [pendingPrompt, selectedConversation, isStreaming, sendTurn]);


  /**
   * Put `text` in front of the athlete in a thread, creating one when none is
   * open. `send` dispatches it as a turn (the invite deep link, the "+" menu's
   * group creation, the info drawer's `/coach remove`); `draft` seeds the
   * composer and lets them finish the line.
   */
  const runComposerAction = useCallback((action: PendingComposerAction) => {
    if (action.kind === 'send' && selectedConversation) {
      void sendTurn(action.text);
      return;
    }
    if (action.kind === 'send') {
      // No thread yet: queue the text and let the pending-prompt effect send
      // it the moment the fresh conversation exists.
      setPendingPrompt(action.text);
      startConversation();
      return;
    }
    if (!selectedConversation) startConversation();
    setNewMessage(action.text);
    inputRef.current?.focus();
  }, [selectedConversation, startConversation, sendTurn]);

  /** Start a fresh thread whose composer already holds `/`, palette open. */
  const handleOpenCommands = useCallback(() => {
    runComposerAction({ kind: 'draft', text: '/' });
  }, [runComposerAction]);

  // The shell's own composer action — the `/groups/join/:code` deep link, which
  // lands on chat and sends `/group join CODE` as its first turn.
  useEffect(() => {
    if (!pendingComposerAction) return;
    runComposerAction(pendingComposerAction);
    onPendingComposerActionConsumed?.();
  }, [pendingComposerAction, runComposerAction, onPendingComposerActionConsumed]);

  // Message action handlers
  const handleCopyMessage = useCallback((content: string) => {
    navigator.clipboard.writeText(content);
    showSuccessToast(t('app.copiedTitle'), t('app.copiedBody'), 2000);
  }, [showSuccessToast, t]);

  /**
   * Press handler for a control the reply's `actions` block carried.
   *
   * A `postback` sends its `value` as the next turn, so the press flows
   * through the exact same pipeline a typed command would. A `url` opens its
   * `value` — but only after {@link trustedActionUrl} vouches for the host:
   * the value reaches the client inside a model-adjacent reply, so an
   * unvouched address is an open redirect wearing a button. A refused URL
   * opens nothing.
   */
  const handleActionClick = useCallback((action: ChatMessageAction) => {
    if (action.action_type === 'url') {
      const target = trustedActionUrl(action.value, [window.location.origin]);
      if (target) window.open(target, '_blank', 'noopener,noreferrer');
      return;
    }
    void sendTurn(action.value);
  }, [sendTurn]);

  const handleShareMessage = useCallback((content: string) => {
    // Use native Web Share API if available, otherwise copy to clipboard
    if (navigator.share) {
      navigator.share({
        title: t('chat.aiInsightLabel'),
        text: content,
      }).catch(() => {
        // User cancelled share, ignore
      });
    } else {
      navigator.clipboard.writeText(content);
      showInfoToast(t('app.copiedTitle'), t('app.messageCopiedForSharing'), 2000);
    }
  }, [showInfoToast, t]);

  // Apply a rating change optimistically and persist it. Clicking the active
  // rating again toggles it off (DELETE); otherwise the rating is upserted.
  // On failure the optimistic change is reverted and an error surfaced.
  const applyFeedback = useCallback(async (messageId: string, rating: 'up' | 'down') => {
    if (!selectedConversation) return;
    const previous = messageFeedback.get(messageId) ?? null;
    const next: MessageFeedback = previous === rating ? null : rating;

    setMessageFeedback(prev => {
      const newMap = new Map(prev);
      if (next) newMap.set(messageId, next);
      else newMap.delete(messageId);
      return newMap;
    });
    // Switching away from a down-rating drops its reason.
    if (next !== 'down') {
      setMessageFeedbackComment(prev => {
        if (!prev.has(messageId)) return prev;
        const newMap = new Map(prev);
        newMap.delete(messageId);
        return newMap;
      });
    }

    try {
      if (next === null) {
        await chatApi.deleteMessageFeedback(selectedConversation, messageId);
      } else {
        await chatApi.submitMessageFeedback(selectedConversation, messageId, next);
      }
    } catch (error) {
      // Revert the optimistic rating on failure.
      setMessageFeedback(prev => {
        const newMap = new Map(prev);
        if (previous) newMap.set(messageId, previous);
        else newMap.delete(messageId);
        return newMap;
      });
      setErrorMessage(error instanceof Error ? error.message : t('chat.feedbackSaveFailed'));
    }
  }, [selectedConversation, messageFeedback, t]);

  const handleThumbsUp = useCallback((messageId: string) => {
    void applyFeedback(messageId, 'up');
  }, [applyFeedback]);

  const handleThumbsDown = useCallback((messageId: string) => {
    void applyFeedback(messageId, 'down');
  }, [applyFeedback]);

  // Persist an optional thumbs-down reason on the existing feedback row. The
  // down rating is already saved; this only adds/updates the comment.
  const handleSubmitFeedbackReason = useCallback(async (messageId: string, comment: string) => {
    if (!selectedConversation) return;
    const trimmed = comment.trim();
    setMessageFeedbackComment(prev => {
      const newMap = new Map(prev);
      if (trimmed) newMap.set(messageId, trimmed);
      else newMap.delete(messageId);
      return newMap;
    });
    try {
      await chatApi.submitMessageFeedback(
        selectedConversation,
        messageId,
        'down',
        trimmed || undefined,
      );
    } catch (error) {
      setErrorMessage(error instanceof Error ? error.message : t('chat.feedbackSaveFailed'));
    }
  }, [selectedConversation, t]);

  /**
   * Retry a failed assistant turn.
   *
   * Drops the failed row from the transcript and re-sends the user message
   * that produced it, so the athlete is left with one attempt rather than a
   * failure followed by its replacement.
   */
  const handleRetryMessage = useCallback(async (messageId: string) => {
    if (!selectedConversation || isStreaming) return;

    const messages = messagesData?.messages || [];
    const messageIndex = messages.findIndex(m => m.id === messageId);
    if (messageIndex <= 0) return;

    // Walk back to the user message that produced this assistant turn.
    let userMessageIndex = messageIndex - 1;
    while (userMessageIndex >= 0 && messages[userMessageIndex].role !== 'user') {
      userMessageIndex--;
    }
    if (userMessageIndex < 0) return;

    queryClient.setQueryData(
      QUERY_KEYS.chat.messages(selectedConversation),
      (old: { messages: Message[] } | undefined) => ({
        messages: (old?.messages ?? []).filter(m => m.id !== messageId),
      }),
    );
    await sendTurn(messages[userMessageIndex].content);
  }, [selectedConversation, isStreaming, messagesData?.messages, queryClient, sendTurn]);

  /** The "+" menu's three items, wired the same way in both header slots. */
  const composeMenu = (withParticipants: boolean) => (
    <ChatComposeMenu
      onNewChat={() => startConversation()}
      onNewGroupChat={(command) => runComposerAction({ kind: 'send', text: command })}
      onAddParticipant={
        withParticipants
          ? () => {
              setInfoOpensParticipants(true);
              setInfoOpen(true);
            }
          : undefined
      }
      disabled={createConversation.isPending}
    />
  );

  /** The thread is gone — deleted, or left with the group it belonged to. */
  const handleThreadGone = () => {
    setInfoOpen(false);
    onSelectConversation(null);
  };

  const handleDeleteConversation = async () => {
    if (!selectedConversation) return;
    await remove(selectedConversation);
    handleThreadGone();
  };

  const banner = showConnectBanner ? (
    <div className="px-4 pt-3 md:px-6">
      <ConnectProviderBanner />
    </div>
  ) : null;

  // The list's "+" knows which thread is open, so on a wide screen it offers
  // the same three ways in as the thread header's "+" beside it.
  const listColumn = (
    <ConversationList
      selectedConversation={selectedConversation}
      onSelectConversation={onSelectConversation}
      compose={composeMenu(Boolean(selectedConversation))}
    />
  );

  const threadPane = !selectedConversation ? (
    <div className="flex flex-1 flex-col overflow-hidden">
      {banner}
      <ChatEmptyState
        compose={composeMenu(false)}
        onOpenCommands={handleOpenCommands}
        disabled={createConversation.isPending}
        onNavigate={onNavigate}
        providerStatus={providerStatusLine}
      />
    </div>
  ) : (
    /* Active Conversation View */
    <div className="flex h-full flex-col">
      <ThreadHeader
        title={headerTitle}
        subtitle={headerSubtitle}
        initials={initialsFor(headerTitle)}
        avatarSlot={activeConversation ? avatarSlot(activeConversation) : 0}
        showBrandMark={Boolean(activeCoachTitle) && !activeConversation?.group_id}
        onOpenInfo={() => {
          setInfoOpensParticipants(false);
          setInfoOpen(true);
        }}
        onBack={isDesktop ? undefined : () => onSelectConversation(null)}
        actions={composeMenu(true)}
      />
      {/* Usage warning banner */}
      <UsageWarningBanner level={usageStatus.level} message={usageStatus.message} />
      {banner}
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="px-4 py-4 md:px-8">
          <MessageList
            messages={messagesData?.messages || []}
            messageMetadata={messageMetadata}
            messageFeedback={messageFeedback}
            messageFeedbackComment={messageFeedbackComment}
            messageBlocks={messageBlocks}
            verdicts={verdicts}
            assistantLabel={activeCoachTitle ?? undefined}
            isLoading={messagesLoading}
            isStreaming={isStreaming}
            streamingContent={streamingContent}
            progressStatusText={progressStatusText}
            errorMessage={errorMessage}
            oauthNotification={oauthNotification}
            onDismissError={() => setErrorMessage(null)}
            onDismissOAuthNotification={() => setOauthNotification(null)}
            onCopyMessage={handleCopyMessage}
            onShareMessage={handleShareMessage}
            onThumbsUp={handleThumbsUp}
            onThumbsDown={handleThumbsDown}
            onSubmitFeedbackReason={handleSubmitFeedbackReason}
            onRetryMessage={handleRetryMessage}
            onShowVerdict={handleShowVerdict}
            onAskAboutClaim={handleAskAboutClaim}
            onActionClick={handleActionClick}
          />
        </div>
      </div>

      <MessageInput
        value={newMessage}
        onChange={setNewMessage}
        onSend={handleSendMessage}
        isStreaming={isStreaming}
        disabled={usageStatus.sendDisabled}
        conversationId={selectedConversation}
      />
    </div>
  );

  return (
    <div className="relative h-full">
      <ChatShell list={listColumn} thread={threadPane} hasSelection={Boolean(selectedConversation)} />

      {/* Drawers */}
      {infoOpen && activeConversation ? (
        <ConversationInfoPanel
          conversation={activeConversation}
          openParticipants={infoOpensParticipants}
          onClose={() => setInfoOpen(false)}
          onSendCommand={(text) => {
            setInfoOpen(false);
            runComposerAction({ kind: 'send', text });
          }}
          onEditCoach={(coachId) => {
            setInfoOpen(false);
            onNavigate?.(`discover/${encodeURIComponent(coachId)}`);
          }}
          onRename={(title) => {
            void rename(activeConversation.id, title);
            setInfoOpen(false);
          }}
          onDelete={() => void handleDeleteConversation()}
          onThreadGone={handleThreadGone}
        />
      ) : null}

      {verdictMessageId ? (
        <VerdictDrawer
          verdicts={drawerVerdicts}
          loading={verdictsFetching && drawerVerdicts.length === 0}
          onClose={() => setVerdictMessageId(null)}
          onAskAboutClaim={(verdict) => {
            handleAskAboutClaim(verdict);
            setVerdictMessageId(null);
          }}
        />
      ) : null}
    </div>
  );
}
