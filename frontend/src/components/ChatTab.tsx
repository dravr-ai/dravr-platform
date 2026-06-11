// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: AI Chat tab component for users to interact with fitness AI assistant
// ABOUTME: Renders chat interface with collapsible conversations panel

import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { ConfirmDialog, TabHeader } from './ui';
import { chatApi, providersApi, coachesApi, oauthApi } from '../services/api';
import { useAuth } from '../hooks/useAuth';
import { useAgUiProgress } from '../hooks/useAgUiProgress';
import PromptSuggestions from './PromptSuggestions';
import { MessageCircle, Plus, Sparkles } from 'lucide-react';
import { createInsightPrompt, stripContextPrefix } from '@pierre/chat-utils';
import {
  MessageList,
  MessageInput,
  ProviderConnectionModal,
  CoachFormModal,
  CreateCoachFromConversationModal,
  DEFAULT_COACH_FORM_DATA,
} from './chat';
import ChatVerdictDrawer from './chat/ChatVerdictDrawer';
import UsageWarningBanner from './chat/UsageWarningBanner';
import { ConnectProviderBanner } from './ConnectProviderBanner';
import { useUsageStatus } from '../hooks/useUsageStatus';
import ShareChatMessageModal from './social/ShareChatMessageModal';
import { useSuccessToast, useInfoToast, useErrorToast } from './ui';
import { QUERY_KEYS } from '../constants/queryKeys';
import type { ChatVerdictRow } from '@pierre/api-client';
import type {
  Message,
  Conversation,
  Coach,
  MessageMetadata,
  MessageActionItem,
  MessageFeedback,
  OAuthNotification,
  CoachDeleteConfirmation,
  PendingCoachAction,
  CoachFormData,
} from './chat';
import type { CreateCoachRequest, UpdateCoachRequest, MessageFeedbackEntry } from '@pierre/shared-types';

/** Convert UI form data to API request, building data_requirements from structured fields */
function formDataToCreateRequest(data: CoachFormData): CreateCoachRequest {
  const request: CreateCoachRequest = {
    title: data.title,
    description: data.description || undefined,
    system_prompt: data.system_prompt,
    category: data.category,
  };

  if (data.startup_query.trim()) {
    request.startup_query = data.startup_query.trim();
  }

  if (data.prefetch_enabled) {
    request.data_requirements = {
      activities: {
        count: data.activity_count,
        sport_types: data.sport_types,
        time_frame: data.time_frame,
        mode: data.detail_mode,
        format: 'toon',
        analysis_type: 'general_overview',
      },
      athlete_profile: data.athlete_profile,
    };
  }

  // Pass structured sections when provided
  if (data.purpose.trim()) request.purpose = data.purpose.trim();
  if (data.when_to_use.trim()) request.when_to_use = data.when_to_use.trim();
  if (data.instructions.trim()) request.instructions = data.instructions.trim();
  if (data.example_inputs.trim()) request.example_inputs = data.example_inputs.trim();
  if (data.example_outputs.trim()) request.example_outputs = data.example_outputs.trim();
  if (data.success_criteria.trim()) request.success_criteria = data.success_criteria.trim();

  return request;
}

/** Convert UI form data to API update request */
function formDataToUpdateRequest(data: CoachFormData): UpdateCoachRequest {
  const request: UpdateCoachRequest = {
    title: data.title,
    description: data.description || undefined,
    system_prompt: data.system_prompt,
    category: data.category,
    startup_query: data.startup_query.trim() || undefined,
  };

  if (data.prefetch_enabled) {
    request.data_requirements = {
      activities: {
        count: data.activity_count,
        sport_types: data.sport_types,
        time_frame: data.time_frame,
        mode: data.detail_mode,
        format: 'toon',
        analysis_type: 'general_overview',
      },
      athlete_profile: data.athlete_profile,
    };
  }

  // Pass structured sections when provided
  if (data.purpose.trim()) request.purpose = data.purpose.trim();
  if (data.when_to_use.trim()) request.when_to_use = data.when_to_use.trim();
  if (data.instructions.trim()) request.instructions = data.instructions.trim();
  if (data.example_inputs.trim()) request.example_inputs = data.example_inputs.trim();
  if (data.example_outputs.trim()) request.example_outputs = data.example_outputs.trim();
  if (data.success_criteria.trim()) request.success_criteria = data.success_criteria.trim();

  return request;
}

interface ChatTabProps {
  selectedConversation: string | null;
  onSelectConversation: (id: string | null) => void;
  onNavigateToInsights?: () => void;
}

export default function ChatTab({ selectedConversation, onSelectConversation, onNavigateToInsights }: ChatTabProps) {
  const queryClient = useQueryClient();
  const { token } = useAuth();
  const showSuccessToast = useSuccessToast();
  const showInfoToast = useInfoToast();
  const showErrorToast = useErrorToast();
  const [newMessage, setNewMessage] = useState('');
  const [isStreaming, setIsStreaming] = useState(false);
  const [streamingContent, setStreamingContent] = useState('');
  // AG-UI run id for the in-flight turn, or `null` between turns. Sent
  // as `agui_run_id` with the chat POST so the backend registers the
  // run; `useAgUiProgress` subscribes to it to surface pipeline
  // progress (e.g. "reading your question…") while the turn streams.
  const [aguiRunId, setAguiRunId] = useState<string | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [errorCountdown, setErrorCountdown] = useState<number | null>(null);
  // Set when the backend rejects messaging with NoProviderConnected. Instead of
  // surfacing a raw 403, we show a friendly connect-provider prompt with a
  // deep link to the Data Providers screen.
  const [needsProvider, setNeedsProvider] = useState(false);
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
  const [activityLists, setActivityLists] = useState<Map<string, string>>(new Map());
  // Slash-command action buttons (e.g. per-coach select on /coach).
  // Keyed by assistant message id; not persisted — buttons disappear
  // when the conversation is reloaded, the text body stays.
  const [messageActions, setMessageActions] = useState<Map<string, MessageActionItem[]>>(new Map());
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
  const { data: providersData } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
  });

  const hasConnectedProvider = providersData?.providers?.some(p => p.connected) ?? false;

  // Fetch messages for selected conversation
  const { data: messagesData, isLoading: messagesLoading } = useQuery<{ messages: Message[]; feedback?: MessageFeedbackEntry[] }>({
    queryKey: QUERY_KEYS.chat.messages(selectedConversation),
    queryFn: () => chatApi.getConversationMessages(selectedConversation!),
    enabled: !!selectedConversation,
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

  // Tier 5.5 — claim verdicts attached to messages in the selected conversation.
  // Refetched alongside messages so a coach reply that triggers verification
  // surfaces its chip without a manual reload.
  const { data: verdictsData } = useQuery({
    queryKey: ['chat', 'verdicts', selectedConversation],
    queryFn: () => chatApi.getConversationVerdicts(selectedConversation!),
    enabled: !!selectedConversation,
  });
  const verdicts: ChatVerdictRow[] = verdictsData?.verdicts ?? [];

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

  // Drawer state for the Tier 5.5 verdict detail surface.
  const [selectedVerdict, setSelectedVerdict] = useState<ChatVerdictRow | null>(null);

  // Live pipeline progress for the in-flight turn. The hook subscribes
  // to the AG-UI SSE stream for `aguiRunId` (Bearer-authed via `token`)
  // and surfaces a short status line ("reading your question…",
  // "calling get_activities…") that renders in the streaming bubble
  // alongside the token-delta text.
  const { statusText: aguiStatusText } = useAgUiProgress(aguiRunId, token);

  const handleAskAboutClaim = useCallback((verdict: ChatVerdictRow) => {
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

  // Handle sending a pending prompt
  useEffect(() => {
    if (pendingPrompt && selectedConversation && !isStreaming) {
      const promptToSend = pendingPrompt;
      setPendingPrompt(null);
      setNewMessage(promptToSend);
      setTimeout(() => {
        const sendButton = document.querySelector('[aria-label="Send message"]') as HTMLButtonElement;
        sendButton?.click();
      }, 100);
    }
  }, [pendingPrompt, selectedConversation, isStreaming]);

  // Error countdown timer
  useEffect(() => {
    if (!errorMessage) {
      setErrorCountdown(null);
      return;
    }
    const match = errorMessage.match(/in (\d+) seconds/);
    if (match) {
      setErrorCountdown(parseInt(match[1], 10));
    }
  }, [errorMessage]);

  useEffect(() => {
    if (errorCountdown === null || errorCountdown <= 0) {
      if (errorCountdown === 0) {
        setErrorMessage(null);
        setErrorCountdown(null);
      }
      return;
    }

    const timer = setInterval(() => {
      setErrorCountdown(prev => {
        if (prev === null || prev <= 1) {
          clearInterval(timer);
          return 0;
        }
        return prev - 1;
      });
    }, 1000);

    return () => clearInterval(timer);
  }, [errorCountdown]);

  // Send message handler
  const handleSendMessage = useCallback(async () => {
    if (!newMessage.trim() || !selectedConversation || isStreaming) return;

    if (connectingProvider) {
      sessionStorage.setItem('pierre_oauth_conversation', selectedConversation);
    }

    const displayContent = newMessage.trim();
    let messageContent = displayContent;
    if (oauthNotification) {
      messageContent = `[Context: I just connected my ${oauthNotification.provider} account successfully] ${displayContent}`;
    } else if (hasConnectedProvider && (!messagesData?.messages || messagesData.messages.length === 0)) {
      const connectedProviders = providersData?.providers?.filter(p => p.connected).map(p =>
        p.display_name
      ).join(', ');
      messageContent = `[Context: I have connected ${connectedProviders}] ${displayContent}`;
    }

    setNewMessage('');
    setIsStreaming(true);
    setStreamingContent('');
    setErrorMessage(null);
    setNeedsProvider(false);

    const userMessageId = `user-${Date.now()}`;
    const tempUserMessage: Message = {
      id: userMessageId,
      role: 'user',
      content: displayContent,
      created_at: new Date().toISOString(),
    };

    queryClient.setQueryData(['chat-messages', selectedConversation], (old: { messages: Message[] } | undefined) => ({
      messages: [...(old?.messages || []), tempUserMessage],
    }));

    // Fresh run id per turn — `useAgUiProgress` subscribes to the
    // matching AG-UI stream while the chat POST below is in flight.
    // Reset to null in the `finally` so the subscription closes once
    // the assistant reply has rendered.
    const runId = crypto.randomUUID();
    setAguiRunId(runId);

    try {
      // Request SSE — the backend streams ACP token deltas + tool-call
      // observations as they arrive and ends with a `done` event carrying
      // the same JSON the blocking branch would have returned. The
      // streaming body keeps nginx (60s `proxy_read_timeout`) from cutting
      // off long turns and lets us render the assistant reply
      // progressively as the model produces it.
      const response = await fetch(`/api/chat/conversations/${selectedConversation}/messages`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Accept: 'text/event-stream',
          'Authorization': `Bearer ${token}`,
          'X-Client-Platform': 'web',
        },
        body: JSON.stringify({ content: messageContent, agui_run_id: runId }),
      });

      if (!response.ok || !response.body) {
        const errorData = await response.json().catch(() => ({}));
        // Providerless users get a structured NoProviderConnected (403). Surface
        // a friendly connect-provider prompt with a deep link instead of a raw
        // "HTTP error" — messaging is gated until a provider is linked.
        if (errorData?.code === 'NoProviderConnected' || errorData?.details?.action === 'connect_provider') {
          setNeedsProvider(true);
          setIsStreaming(false);
          setStreamingContent('');
          // The gate rejects before persisting, so drop the optimistic user
          // bubble — it was never saved.
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.messages(selectedConversation) });
          return;
        }
        throw new Error(errorData.message || errorData.error || `HTTP error! status: ${response.status}`);
      }

      let assembled = '';
      type StreamAssistantMessage = {
        id?: string;
        role?: 'user' | 'assistant' | 'system';
        content?: string;
        token_count?: number;
        created_at?: string;
      };
      type StreamFinalPayload = {
        assistant_message?: StreamAssistantMessage;
        model?: string;
        execution_time_ms?: number;
        activity_list?: string;
        actions?: MessageActionItem[];
      };
      let finalData: StreamFinalPayload | null = null;

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';

      const dispatch = (eventName: string, dataJson: string) => {
        if (eventName === 'delta') {
          try {
            const parsed = JSON.parse(dataJson) as { delta?: string };
            if (parsed.delta) {
              assembled += parsed.delta;
              setStreamingContent(assembled);
            }
          } catch {
            // Ignore malformed delta payloads — keep the stream alive.
          }
        } else if (eventName === 'done') {
          try {
            finalData = JSON.parse(dataJson);
          } catch {
            finalData = null;
          }
        } else if (eventName === 'error') {
          let message: string;
          try {
            const parsed = JSON.parse(dataJson) as { error?: string };
            message = parsed.error ?? 'Server error';
          } catch {
            // Bad JSON — fall back to a generic surface message.
            message = 'Server error';
          }
          throw new Error(message);
        }
        // Other events (tool_call, keepalive) — ignore for now; no UX yet.
      };

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });

        let sep: number;
        while ((sep = buffer.indexOf('\n\n')) !== -1) {
          const chunk = buffer.slice(0, sep);
          buffer = buffer.slice(sep + 2);
          let eventName = 'message';
          const dataLines: string[] = [];
          for (const line of chunk.split('\n')) {
            if (line.startsWith('event:')) {
              eventName = line.slice(line[6] === ' ' ? 7 : 6).trim();
            } else if (line.startsWith('data:')) {
              dataLines.push(line.slice(line[5] === ' ' ? 6 : 5));
            }
          }
          if (dataLines.length > 0) {
            dispatch(eventName, dataLines.join('\n'));
          }
        }
      }

      const data: StreamFinalPayload = finalData ?? {};
      const assistantMessageId = data.assistant_message?.id || '';
      const model = data.model || '';
      const executionTimeMs = data.execution_time_ms || 0;

      if (assistantMessageId && model) {
        setMessageMetadata(prev => {
          const newMap = new Map(prev);
          newMap.set(assistantMessageId, { model, executionTimeMs });
          return newMap;
        });
      }

      if (data.activity_list && assistantMessageId) {
        setActivityLists(prev => {
          const newMap = new Map(prev);
          newMap.set(assistantMessageId, data.activity_list as string);
          return newMap;
        });
      }

      if (Array.isArray(data.actions) && data.actions.length > 0 && assistantMessageId) {
        setMessageActions(prev => {
          const newMap = new Map(prev);
          newMap.set(assistantMessageId, data.actions as MessageActionItem[]);
          return newMap;
        });
      }

      // Inject the persisted assistant message into the cache directly so
      // the streaming bubble can be cleared in the same batch — invalidating
      // and waiting for the refetch leaves a window where both the streaming
      // bubble (assembled deltas) and the just-arrived persisted message
      // render together, surfacing as a duplicated opening sentence. The
      // server's `done` payload mirrors `ChatCompletionResponse`, so the
      // assistant_message it carries already includes any post-processing
      // additions the refetch would have brought in.
      const assistantMessage = data.assistant_message;
      if (assistantMessage?.id && assistantMessage.content) {
        const persisted: Message = {
          id: assistantMessage.id,
          role: assistantMessage.role ?? 'assistant',
          content: assistantMessage.content,
          token_count: assistantMessage.token_count,
          created_at: assistantMessage.created_at ?? new Date().toISOString(),
        };
        queryClient.setQueryData(
          ['chat-messages', selectedConversation],
          (old: { messages: Message[] } | undefined) => {
            const existing = old?.messages ?? [];
            if (existing.some(m => m.id === persisted.id)) {
              return { messages: existing };
            }
            return { messages: [...existing, persisted] };
          },
        );
      }

      setIsStreaming(false);
      setStreamingContent('');

      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Failed to send message';
      setErrorMessage(message);
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.messages(selectedConversation) });
      setIsStreaming(false);
      setStreamingContent('');
    } finally {
      // Closing the run id collapses the AG-UI progress strip; the
      // streamed assistant reply is now the source of truth.
      setAguiRunId(null);
      usageStatus.invalidate();
    }
  }, [newMessage, selectedConversation, isStreaming, connectingProvider, oauthNotification, hasConnectedProvider, messagesData?.messages, providersData?.providers, queryClient, token, usageStatus]);

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
    const dr = coach.data_requirements;
    setCoachFormData({
      title: coach.title,
      description: coach.description || '',
      system_prompt: coach.system_prompt,
      category: coach.category,
      startup_query: coach.startup_query || '',
      prefetch_enabled: !!dr?.activities,
      activity_count: dr?.activities?.count ?? 20,
      sport_types: dr?.activities?.sport_types ?? [],
      time_frame: dr?.activities?.time_frame ?? '12w',
      detail_mode: (dr?.activities?.mode as 'summary' | 'detailed') ?? 'summary',
      athlete_profile: dr?.athlete_profile ?? false,
      purpose: coach.purpose || '',
      when_to_use: coach.when_to_use || '',
      instructions: coach.instructions || '',
      example_inputs: coach.example_inputs || '',
      example_outputs: coach.example_outputs || '',
      success_criteria: coach.success_criteria || '',
    });
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
    navigator.clipboard.writeText(stripContextPrefix(content));
    showSuccessToast('Copied', 'Message copied to clipboard', 2000);
  }, [showSuccessToast]);

  /**
   * Click handler for a command action button (e.g. the per-coach select
   * buttons on `/coach`). Postback actions seed the composer with the
   * button's `value` (usually another slash command like
   * `/coach select <uuid>`) and immediately dispatch, so the click flows
   * through the exact same pipeline a typed command would.
   */
  const handleActionClick = useCallback((action: MessageActionItem) => {
    if (action.action_type !== 'postback') {
      // URL actions and unknown types are ignored for now — the server
      // currently only produces postback actions from /coach, /group.
      return;
    }
    setNewMessage(action.value);
    // Defer one tick so React commits setNewMessage before handleSendMessage
    // reads it back on the next render cycle.
    setTimeout(() => {
      void handleSendMessage();
    }, 0);
  }, [handleSendMessage]);

  const handleShareMessage = useCallback((content: string) => {
    // Use native Web Share API if available, otherwise copy to clipboard
    const strippedContent = stripContextPrefix(content);
    if (navigator.share) {
      navigator.share({
        title: 'Dravr AI Insight',
        text: strippedContent,
      }).catch(() => {
        // User cancelled share, ignore
      });
    } else {
      navigator.clipboard.writeText(strippedContent);
      showInfoToast('Copied', 'Message copied to clipboard for sharing', 2000);
    }
  }, [showInfoToast]);

  const handleShareToFeed = useCallback((content: string) => {
    setShareToFeedContent(stripContextPrefix(content));
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

    try {
      // Send the insight prompt via the chat API (will appear in chat as response)
      const response = await fetch(`/api/chat/conversations/${selectedConversation}/messages`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Authorization': `Bearer ${token}`,
        },
        body: JSON.stringify({ content: insightPrompt }),
      });

      if (!response.ok) {
        const errorData = await response.json().catch(() => ({}));
        // Providerless users get a structured NoProviderConnected (403). Surface
        // a friendly connect-provider prompt with a deep link instead of a raw
        // "HTTP error" — messaging is gated until a provider is linked.
        if (errorData?.code === 'NoProviderConnected' || errorData?.details?.action === 'connect_provider') {
          setNeedsProvider(true);
          setIsStreaming(false);
          setStreamingContent('');
          // The gate rejects before persisting, so drop the optimistic user
          // bubble — it was never saved.
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.messages(selectedConversation) });
          return;
        }
        throw new Error(errorData.message || errorData.error || `HTTP error! status: ${response.status}`);
      }

      // Refresh messages to show the generated insight
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.messages(selectedConversation) });
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Failed to generate insight';
      setErrorMessage(message);
    } finally {
      setIsGeneratingInsight(false);
      setIsStreaming(false);
      setStreamingContent('');
    }
  }, [isGeneratingInsight, selectedConversation, isStreaming, queryClient, token]);

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

  const handleRetryMessage = useCallback(async (messageId: string) => {
    if (!selectedConversation || isStreaming) return;

    // Find the message to retry and get the preceding user message
    const messages = messagesData?.messages || [];
    const messageIndex = messages.findIndex(m => m.id === messageId);
    if (messageIndex <= 0) return;

    // Find the user message that preceded this assistant message
    let userMessageIndex = messageIndex - 1;
    while (userMessageIndex >= 0 && messages[userMessageIndex].role !== 'user') {
      userMessageIndex--;
    }
    if (userMessageIndex < 0) return;

    const userMessage = messages[userMessageIndex];
    setNewMessage(userMessage.content);

    // Use setTimeout to allow state update before triggering send
    setTimeout(() => {
      const sendButton = document.querySelector('[aria-label="Send message"]') as HTMLButtonElement;
      sendButton?.click();
    }, 100);
  }, [selectedConversation, isStreaming, messagesData?.messages]);

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
        {/* Providerless users hit a NoProviderConnected 403 on send; show the
            connect-provider nudge (deep-links to Data Providers) instead of a
            raw error. Rendered above both the welcome and conversation views so
            it appears regardless of conversation state. Self-hides once a
            provider is connected. */}
        {needsProvider && (
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

          {/* Conversation Header: active coach name (left) + Create Coach (right) */}
          {(activeCoachTitle || (messagesData?.messages?.length ?? 0) >= 2) && (
            <div className="border-b ghost-border px-4 md:px-6 py-3 flex items-center justify-between gap-3">
              <div className="min-w-0 flex items-center gap-2">
                {activeCoachTitle && (
                  <>
                    <img src="/dravr-icon.svg" alt="" className="w-5 h-5 rounded-md flex-shrink-0" />
                    <span className="text-sm font-semibold text-on-surface truncate" title={activeCoachTitle}>
                      {activeCoachTitle}
                    </span>
                  </>
                )}
              </div>
              {(messagesData?.messages?.length ?? 0) >= 2 && (
                <button
                  onClick={() => setShowCreateCoachFromConversation(true)}
                  disabled={isStreaming}
                  className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium text-primary bg-pierre-violet/10 hover:bg-pierre-violet/20 rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex-shrink-0"
                  title="Create a coach based on this conversation"
                >
                  <Sparkles className="w-3.5 h-3.5" />
                  Create Coach
                </button>
              )}
            </div>
          )}
          <div className="flex-1 overflow-y-auto min-h-0">
            <div className="max-w-3xl mx-auto py-4 md:py-6 px-4 md:px-6">
              <MessageList
                messages={messagesData?.messages || []}
                messageMetadata={messageMetadata}
                messageFeedback={messageFeedback}
                messageFeedbackComment={messageFeedbackComment}
                activityLists={activityLists}
                messageActions={messageActions}
                insightMessageIds={new Set<string>()}
                verdicts={verdicts}
                isLoading={messagesLoading}
                isStreaming={isStreaming}
                streamingContent={streamingContent}
                progressStatusText={aguiStatusText}
                errorMessage={errorMessage}
                errorCountdown={errorCountdown}
                oauthNotification={oauthNotification}
                onDismissError={() => { setErrorMessage(null); setErrorCountdown(null); }}
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
        <ChatVerdictDrawer
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
