// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: AI Chat tab component for users to interact with fitness AI assistant
// ABOUTME: Renders chat interface with collapsible conversations panel

import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { ConfirmDialog, TabHeader } from './ui';
import { chatApi, providersApi, coachesApi, oauthApi } from '../services/api';
import { holdIdleWhileBusy, idleSignal } from '../services/api/idleSignal';
import { track } from '../services/analytics';
import PromptSuggestions from './PromptSuggestions';
import { MessageCircle, Plus, Sparkles } from 'lucide-react';
import {
  createInsightPrompt,
  statusTextForProgress,
  trustedActionUrl,
} from '@pierre/chat-utils';
import {
  MessageList,
  MessageInput,
  ProviderConnectionModal,
  CoachFormModal,
  CreateCoachFromConversationModal,
  ConversationParticipants,
  DEFAULT_COACH_FORM_DATA,
  coachToFormData,
  formDataToCreateRequest,
  formDataToUpdateRequest,
} from './chat';
import VerdictDrawer from './chat/VerdictDrawer';
import UsageWarningBanner from './chat/UsageWarningBanner';
import { ConnectProviderBanner } from './ConnectProviderBanner';
import { useUsageStatus } from '../hooks/useUsageStatus';
import ShareChatMessageModal from './social/ShareChatMessageModal';
import { useSuccessToast, useInfoToast, useErrorToast } from './ui';
import { QUERY_KEYS } from '../constants/queryKeys';
import { replySceneBlocks } from '@pierre/api-client';
import type { ChatMessageAction, ClaimVerdict, ReplyBlock } from '@pierre/shared-types';
import type {
  Message,
  Conversation,
  Coach,
  MessageMetadata,
  MessageFeedback,
  OAuthNotification,
  CoachDeleteConfirmation,
  PendingCoachAction,
  CoachFormData,
} from './chat';
import type { MessageFeedbackEntry } from '@pierre/shared-types';

interface ChatTabProps {
  selectedConversation: string | null;
  onSelectConversation: (id: string | null) => void;
  onNavigateToInsights?: () => void;
}

export default function ChatTab({ selectedConversation, onSelectConversation, onNavigateToInsights }: ChatTabProps) {
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
  const [showIdeas, setShowIdeas] = useState(false);
  const [connectingProvider, setConnectingProvider] = useState<string | null>(null);
  const [showProviderModal, setShowProviderModal] = useState(false);
  const [pendingCoachAction, setPendingCoachAction] = useState<PendingCoachAction | null>(null);
  const [messageMetadata, setMessageMetadata] = useState<Map<string, MessageMetadata>>(new Map());
  const [messageFeedback, setMessageFeedback] = useState<Map<string, MessageFeedback>>(new Map());
  // Saved thumbs-down reasons, keyed by message id. Hydrated from the server
  // alongside the ratings so a reload re-renders the comment.
  const [messageFeedbackComment, setMessageFeedbackComment] = useState<Map<string, string>>(new Map());
  // What the server decided this turn draws, keyed by assistant message id.
  // Not persisted: a reloaded conversation has no block list on the wire, so
  // its rows are decoded back into the same shape by the renderer.
  const [messageBlocks, setMessageBlocks] = useState<Map<string, ReplyBlock[]>>(new Map());
  const [showCoachModal, setShowCoachModal] = useState(false);
  const [editingCoachId, setEditingCoachId] = useState<string | null>(null);
  const [coachFormData, setCoachFormData] = useState<CoachFormData>(DEFAULT_COACH_FORM_DATA);
  const [coachDeleteConfirmation, setCoachDeleteConfirmation] = useState<CoachDeleteConfirmation | null>(null);
  const [showCreateCoachFromConversation, setShowCreateCoachFromConversation] = useState(false);
  const [showShareToFeedModal, setShowShareToFeedModal] = useState(false);
  const [shareToFeedContent, setShareToFeedContent] = useState('');
  const [isGeneratingInsight, setIsGeneratingInsight] = useState(false);

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
  const { data: verdictsData } = useQuery({
    queryKey: ['chat', 'verdicts', selectedConversation],
    queryFn: () => chatApi.getConversationVerdicts(selectedConversation!),
    enabled: !!selectedConversation,
  });
  const verdicts: ClaimVerdict[] = verdictsData?.verdicts ?? [];

  // Conversations and coaches power the coach-aware chat header and the
  // assistant author label. The conversation carries `coach_id`; the coach
  // title is joined from the coaches list.
  const { data: conversationsData } = useQuery({
    queryKey: QUERY_KEYS.chat.conversations(),
    queryFn: () => chatApi.getConversations(),
  });
  const { data: coachesListData } = useQuery<{ coaches: Coach[] }>({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
    // Coach titles change rarely; cache so the header doesn't refetch on
    // every conversation switch.
    staleTime: 5 * 60 * 1000,
  });

  // Title of the coach attached to the active conversation, or null when the
  // chat has no coach. `pendingCoachId` covers a freshly created conversation
  // whose `coach_id` has not yet been written back to the conversation list.
  const activeCoachTitle = useMemo<string | null>(() => {
    const conversation = conversationsData?.conversations?.find(
      c => c.id === selectedConversation,
    );
    const coachId = conversation?.coach_id ?? pendingCoachId;
    if (!coachId) return null;
    return coachesListData?.coaches?.find(c => c.id === coachId)?.title ?? null;
  }, [conversationsData, coachesListData, selectedConversation, pendingCoachId]);

  // What the header names when no coach is attached: the conversation's own
  // title, or a placeholder for a thread that has not been titled yet. The
  // header always renders so the participants control has a home; this keeps
  // its left side from being an empty rule.
  const headerTitle = useMemo<string>(() => {
    if (activeCoachTitle) return activeCoachTitle;
    const conversation = conversationsData?.conversations?.find(
      c => c.id === selectedConversation,
    );
    return conversation?.title?.trim() || 'New conversation';
  }, [activeCoachTitle, conversationsData, selectedConversation]);

  // Drawer state for the claim verdict detail surface.
  const [selectedVerdict, setSelectedVerdict] = useState<ClaimVerdict | null>(null);

  const handleAskAboutClaim = useCallback((verdict: ClaimVerdict) => {
    setNewMessage(
      `Can you back up this claim with evidence? "${verdict.claim_text}"`,
    );
  }, []);

  // Mutations. Takes an optional coach ID; the server resolves the
  // coach's system prompt at runtime from the coaches table.
  const createConversation = useMutation<{ id: string }, Error, string | void>({
    mutationFn: (coachId) => {
      const now = new Date();
      const defaultTitle = `Chat ${now.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })} ${now.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' })}`;
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
          'Conversation limit reached',
          `You've reached your limit${limit ? ` of ${limit}` : ''} active conversations. Delete or archive one from the sidebar to start a new chat.`
        );
        return;
      }
      showErrorToast(
        'Could not start chat',
        'Something went wrong creating the conversation. Please try again.'
      );
    },
  });

  const createCoach = useMutation({
    mutationFn: (data: CoachFormData) => coachesApi.create(formDataToCreateRequest(data)),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      setShowCoachModal(false);
      setCoachFormData(DEFAULT_COACH_FORM_DATA);
    },
  });

  const updateCoach = useMutation({
    mutationFn: ({ id, data }: { id: string; data: CoachFormData }) => coachesApi.update(id, formDataToUpdateRequest(data)),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      setShowCoachModal(false);
      setEditingCoachId(null);
      setCoachFormData(DEFAULT_COACH_FORM_DATA);
    },
  });

  const deleteCoach = useMutation({
    mutationFn: (id: string) => coachesApi.delete(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      setCoachDeleteConfirmation(null);
    },
  });

  // Restore message metadata (model label) from conversation when loading existing messages
  useEffect(() => {
    if (!selectedConversation || !messagesData?.messages?.length) return;
    const conversationsData = queryClient.getQueryData<{ conversations: Conversation[] }>(
      QUERY_KEYS.chat.conversations()
    );
    const conversation = conversationsData?.conversations?.find(c => c.id === selectedConversation);
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

  // Focus input when conversation is selected
  useEffect(() => {
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
          const savedCoachAction = sessionStorage.getItem('pierre_pending_coach_action');

          if (savedConversation) sessionStorage.removeItem('pierre_oauth_conversation');
          if (savedCoachAction) sessionStorage.removeItem('pierre_pending_coach_action');

          return {
            result,
            savedConversation,
            savedCoachAction: savedCoachAction ? JSON.parse(savedCoachAction) : null,
          };
        } else if (result.timestamp <= fiveMinutesAgo) {
          sessionStorage.removeItem('pierre_oauth_conversation');
          sessionStorage.removeItem('pierre_pending_coach_action');
        }
      } catch {
        // Ignore parse errors
      }
      return null;
    };

    const processOAuthData = (data: { result: { provider: string }; savedConversation: string | null; savedCoachAction: PendingCoachAction | null }) => {
      if (isProcessingOAuth) return;
      isProcessingOAuth = true;

      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.oauth.status() });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.profile() });

      const providerDisplay = data.result.provider.charAt(0).toUpperCase() + data.result.provider.slice(1);
      setOauthNotification({ provider: providerDisplay, timestamp: Date.now() });
      setConnectingProvider(null);

      if (data.savedConversation) {
        onSelectConversation(data.savedConversation);
      }

      if (data.savedCoachAction) {
        setPendingPrompt(data.savedCoachAction.prompt);
        if (data.savedCoachAction.coachId) {
          setPendingCoachId(data.savedCoachAction.coachId);
        }
        createConversation.mutate(data.savedCoachAction.coachId);
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
          const savedCoachActionStr = sessionStorage.getItem('pierre_pending_coach_action');

          if (savedConversation) sessionStorage.removeItem('pierre_oauth_conversation');
          if (savedCoachActionStr) sessionStorage.removeItem('pierre_pending_coach_action');

          let savedCoachAction = null;
          if (savedCoachActionStr) {
            try {
              savedCoachAction = JSON.parse(savedCoachActionStr);
            } catch {
              // Ignore parse errors
            }
          }

          processOAuthData({
            result: { provider },
            savedConversation,
            savedCoachAction,
          });
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
  }, [queryClient, createConversation, onSelectConversation]);

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

    if (connectingProvider) {
      sessionStorage.setItem('pierre_oauth_conversation', selectedConversation);
    }

    setIsStreaming(true);
    setStreamingContent('');
    setErrorMessage(null);
    track({ name: 'feature_engaged', props: { feature: 'chat_message_sent' } });

    const userMessageId = `user-${Date.now()}`;
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
        const text = statusTextForProgress(progress);
        if (text !== null) setProgressStatusText(text);
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
  }, [selectedConversation, isStreaming, connectingProvider, queryClient, usageStatus]);

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


  // Coach handlers. The optional coachId is what we now propagate to the
  // conversation record — the server resolves the coach's system prompt.
  const handleSelectPrompt = (prompt: string, coachId?: string) => {
    if (!hasConnectedProvider) {
      setPendingCoachAction({ prompt, coachId });
      sessionStorage.setItem('pierre_pending_coach_action', JSON.stringify({ prompt, coachId }));
      setShowProviderModal(true);
      return;
    }

    setPendingPrompt(prompt);
    if (coachId) {
      setPendingCoachId(coachId);
    }
    createConversation.mutate(coachId);
  };

  const handleFillPrompt = (prompt: string) => {
    setNewMessage(prompt);
    setShowIdeas(false);
    inputRef.current?.focus();
  };

  const handleEditCoach = (coach: Coach) => {
    setEditingCoachId(coach.id);
    setCoachFormData(coachToFormData(coach));
    setShowCoachModal(true);
  };

  const handleDeleteCoach = (coach: Coach) => {
    setCoachDeleteConfirmation({ id: coach.id, title: coach.title });
  };

  const handleConfirmCoachDelete = () => {
    if (coachDeleteConfirmation) {
      deleteCoach.mutate(coachDeleteConfirmation.id);
    }
  };

  const handleConnectProvider = async (provider: string) => {
    // Mobile Safari requires window.open to fire inside the synchronous
    // user-gesture call stack. Pre-open a blank window so the popup permission
    // is captured before the async authorize-URL fetch.
    const popup = window.open('about:blank', '_blank');
    setConnectingProvider(provider);
    if (selectedConversation) {
      sessionStorage.setItem('pierre_oauth_conversation', selectedConversation);
    }
    try {
      const authUrl = await oauthApi.getAuthorizeUrlForProvider(provider);
      if (popup && !popup.closed) {
        popup.location.href = authUrl;
      } else {
        window.location.href = authUrl;
        return;
      }
      // Clear the per-card spinner once the OAuth tab is loading. The chat
      // OAuth handler resolves the eventual success through localStorage;
      // holding the spinner here would strand the card if the user closes
      // the OAuth tab without finishing.
      setConnectingProvider(null);
    } catch (error) {
      if (popup && !popup.closed) {
        popup.close();
      }
      console.error(`Failed to get OAuth URL for ${provider}:`, error);
      setConnectingProvider(null);
    }
  };

  // Message action handlers
  const handleCopyMessage = useCallback((content: string) => {
    navigator.clipboard.writeText(content);
    showSuccessToast('Copied', 'Message copied to clipboard', 2000);
  }, [showSuccessToast]);

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
        title: 'Dravr AI Insight',
        text: content,
      }).catch(() => {
        // User cancelled share, ignore
      });
    } else {
      navigator.clipboard.writeText(content);
      showInfoToast('Copied', 'Message copied to clipboard for sharing', 2000);
    }
  }, [showInfoToast]);

  const handleShareToFeed = useCallback((content: string) => {
    setShareToFeedContent(content);
    setShowShareToFeedModal(true);
  }, []);

  const handleCreateInsight = useCallback(async (content: string) => {
    if (isGeneratingInsight || !selectedConversation || isStreaming) return;

    setIsGeneratingInsight(true);
    setIsStreaming(true); // Show "Thinking..." indicator
    setStreamingContent('');
    setErrorMessage(null);

    // Create the insight prompt (will be hidden from display by the filter)
    const insightPrompt = createInsightPrompt(content);

    // The same transport as every other turn. This send used to build its own
    // request with its own header set — no client-platform, no AG-UI run —
    // which is how a header the server added reached the chat composer and
    // not this button.
    await chatApi.sendTurn(selectedConversation, insightPrompt, {
      // Refresh messages to show the generated insight. Insights are one-shot
      // JSON on a self-served path, so the reply is read back from history
      // rather than folded in from the envelope.
      onDone: () => {
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.messages(selectedConversation) });
      },
      onError: error => setErrorMessage(error.message),
    });

    setIsGeneratingInsight(false);
    setIsStreaming(false);
    setStreamingContent('');
  }, [isGeneratingInsight, selectedConversation, isStreaming, queryClient]);

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
      setErrorMessage(error instanceof Error ? error.message : 'Failed to save feedback');
    }
  }, [selectedConversation, messageFeedback]);

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
      setErrorMessage(error instanceof Error ? error.message : 'Failed to save feedback');
    }
  }, [selectedConversation]);

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

  const handleProviderModalClose = () => {
    setShowProviderModal(false);
    setPendingCoachAction(null);
    sessionStorage.removeItem('pierre_pending_coach_action');
  };

  const handleProviderModalSkip = () => {
    setShowProviderModal(false);
    if (pendingCoachAction) {
      setPendingPrompt(pendingCoachAction.prompt);
      if (pendingCoachAction.coachId) {
        setPendingCoachId(pendingCoachAction.coachId);
      }
      createConversation.mutate(pendingCoachAction.coachId);
    }
    setPendingCoachAction(null);
    sessionStorage.removeItem('pierre_pending_coach_action');
  };

  // Coach form handlers
  const handleCoachFormSubmit = () => {
    if (editingCoachId) {
      updateCoach.mutate({ id: editingCoachId, data: coachFormData });
    } else {
      createCoach.mutate(coachFormData);
    }
  };

  const handleCoachFormClose = () => {
    setShowCoachModal(false);
    setEditingCoachId(null);
    setCoachFormData(DEFAULT_COACH_FORM_DATA);
  };

  return (
    <div className="h-full flex bg-surface relative">
      {/* Main Content Area - conversations are now in Dashboard sidebar */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* The nudge used to appear only after a send was refused with a 403.
            That refusal is gone — a providerless athlete now gets a real coach
            reply that says what it cannot see — so the banner is driven by
            provider state directly and shows before they ask, not after they
            are turned away. Self-hides once a provider is connected. */}
        {showConnectBanner && (
          <div className="px-4 md:px-6 pt-3">
            <ConnectProviderBanner />
          </div>
        )}
        {!selectedConversation ? (
          /* Welcome View */
          <div className="flex-1 flex flex-col overflow-hidden">
            <TabHeader
              icon={<MessageCircle className="w-5 h-5" />}
              gradient="boreal-hero-gradient"
              description={
                hasConnectedProvider
                  ? providersData?.providers?.filter(p => p.connected).map(p =>
                      p.display_name
                    ).join(', ') + ' connected'
                  : 'No provider connected'
              }
              actions={
                <button
                  onClick={() => createConversation.mutate()}
                  disabled={createConversation.isPending}
                  className="rounded-lg text-on-primary bg-primary hover:bg-primary-container transition-colors shadow-ambient hover:shadow-ambient disabled:opacity-50 min-w-[44px] min-h-[44px] flex items-center justify-center"
                  title="New Chat"
                  aria-label="New Chat"
                >
                  <Plus className="w-4 h-4" />
                </button>
              }
            />

            <div className="flex-1 overflow-y-auto">
              <div className="w-full max-w-5xl mx-auto px-4 sm:px-6 py-6 sm:py-8">
                <div className="text-center mb-6 sm:mb-8">
                  <h2 className="text-h2-mobile sm:text-2xl text-on-surface mb-2 text-balance">Ready to analyze your fitness</h2>
                  <p className="text-on-surface-variant text-sm">
                    {hasConnectedProvider
                      ? 'Get personalized insights from your activity data'
                    : 'Or ask a question to Dravr'}
                </p>

                <form
                  onSubmit={(e) => {
                    e.preventDefault();
                    if (newMessage.trim()) {
                      setPendingPrompt(newMessage.trim());
                      createConversation.mutate();
                    }
                  }}
                  className="relative mt-6 max-w-2xl mx-auto flex items-stretch gap-2 min-w-0"
                >
                  <input
                    type="text"
                    value={newMessage}
                    onChange={(e) => setNewMessage(e.target.value)}
                    placeholder="Message Dravr..."
                    aria-label="Message Dravr"
                    className="flex-1 min-w-0 rounded-xl border ghost-border bg-surface-container-low text-on-surface placeholder:text-outline px-4 py-3.5 focus:outline-none focus:ring-2 focus:ring-primary/30 focus:border-primary text-sm transition-colors"
                    disabled={createConversation.isPending}
                  />
                  <button
                    type="submit"
                    aria-label="Send message"
                    disabled={!newMessage.trim() || createConversation.isPending}
                    className="flex-shrink-0 min-w-[44px] min-h-[44px] px-3 sm:px-4 bg-primary text-on-primary text-sm font-medium rounded-lg hover:bg-primary-container transition-colors disabled:opacity-50 disabled:cursor-not-allowed inline-flex items-center justify-center gap-1.5"
                  >
                    {createConversation.isPending ? (
                      <div className="pierre-spinner w-4 h-4 border-white border-t-transparent" />
                    ) : (
                      <>
                        <span className="hidden sm:inline">Send</span>
                        <svg className="w-4 h-4" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M14 5l7 7m0 0l-7 7m7-7H3" />
                        </svg>
                      </>
                    )}
                  </button>
                </form>
              </div>

              <PromptSuggestions
                onSelectPrompt={handleSelectPrompt}
                onEditCoach={handleEditCoach}
                onDeleteCoach={handleDeleteCoach}
              />
            </div>
          </div>
        </div>
      ) : (
        /* Active Conversation View */
        <div className="h-full flex flex-col">
          {/* Usage warning banner */}
          <UsageWarningBanner level={usageStatus.level} message={usageStatus.message} />

          {/* Conversation Header: coach or conversation title (left) + participants / Create Coach (right) */}
          <div className="border-b ghost-border px-4 md:px-6 py-3 flex items-center justify-between gap-3">
            <div className="min-w-0 flex items-center gap-2">
              {activeCoachTitle && (
                <img src="/dravr-icon.svg" alt="" className="w-5 h-5 rounded-md flex-shrink-0" />
              )}
              <span
                className="text-sm font-semibold text-on-surface truncate"
                title={headerTitle}
                data-testid="conversation-header-title"
              >
                {headerTitle}
              </span>
            </div>
            <div className="flex items-center gap-2 flex-shrink-0">
              <ConversationParticipants conversationId={selectedConversation} />
              {(messagesData?.messages?.length ?? 0) >= 2 && (
                <button
                  onClick={() => setShowCreateCoachFromConversation(true)}
                  disabled={isStreaming}
                  className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium text-primary bg-primary/10 hover:bg-primary/20 rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex-shrink-0"
                  title="Create a coach based on this conversation"
                >
                  <Sparkles className="w-3.5 h-3.5" />
                  Create Coach
                </button>
              )}
            </div>
          </div>
          <div className="flex-1 overflow-y-auto min-h-0">
            <div className="max-w-3xl mx-auto py-4 md:py-6 px-4 md:px-6">
              <MessageList
                messages={messagesData?.messages || []}
                messageMetadata={messageMetadata}
                messageFeedback={messageFeedback}
                messageFeedbackComment={messageFeedbackComment}
                messageBlocks={messageBlocks}
                insightMessageIds={new Set<string>()}
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
                onShareToFeed={handleShareToFeed}
                onCreateInsight={handleCreateInsight}
                onThumbsUp={handleThumbsUp}
                onThumbsDown={handleThumbsDown}
                onSubmitFeedbackReason={handleSubmitFeedbackReason}
                onRetryMessage={handleRetryMessage}
                onShowVerdict={setSelectedVerdict}
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
            showIdeas={showIdeas}
            onToggleIdeas={() => setShowIdeas(!showIdeas)}
            onSelectPrompt={handleFillPrompt}
            conversationId={selectedConversation}
          />
        </div>
      )}
      </div>

      {/* Modals and Dialogs */}
      <ProviderConnectionModal
        isOpen={showProviderModal}
        onClose={handleProviderModalClose}
        onConnectProvider={handleConnectProvider}
        connectingProvider={connectingProvider}
        onSkip={handleProviderModalSkip}
        isSkipPending={createConversation.isPending}
      />

      <CoachFormModal
        isOpen={showCoachModal}
        isEditing={!!editingCoachId}
        formData={coachFormData}
        onFormDataChange={setCoachFormData}
        onSubmit={handleCoachFormSubmit}
        onClose={handleCoachFormClose}
        isSubmitting={editingCoachId ? updateCoach.isPending : createCoach.isPending}
        submitError={createCoach.isError || updateCoach.isError}
      />

      <ConfirmDialog
        isOpen={!!coachDeleteConfirmation}
        onClose={() => setCoachDeleteConfirmation(null)}
        onConfirm={handleConfirmCoachDelete}
        title="Delete Coach"
        message={`Are you sure you want to delete "${coachDeleteConfirmation?.title || 'this coach'}"? This action cannot be undone.`}
        confirmLabel="Delete"
        cancelLabel="Cancel"
        variant="danger"
        isLoading={deleteCoach.isPending}
      />

      {selectedConversation && (
        <CreateCoachFromConversationModal
          isOpen={showCreateCoachFromConversation}
          conversationId={selectedConversation}
          messageCount={messagesData?.messages?.length ?? 0}
          onClose={() => setShowCreateCoachFromConversation(false)}
          onSuccess={() => {
            setShowCreateCoachFromConversation(false);
            queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
          }}
        />
      )}

      <ShareChatMessageModal
        isOpen={showShareToFeedModal}
        onClose={() => setShowShareToFeedModal(false)}
        content={shareToFeedContent}
        onSuccess={() => {
          setShowShareToFeedModal(false);
          onNavigateToInsights?.();
        }}
      />

      {selectedVerdict ? (
        <VerdictDrawer
          verdict={selectedVerdict}
          onClose={() => setSelectedVerdict(null)}
          onAskAboutClaim={() => {
            handleAskAboutClaim(selectedVerdict);
            setSelectedVerdict(null);
          }}
        />
      ) : null}
    </div>
  );
}
