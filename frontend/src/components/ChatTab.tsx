// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: AI Chat tab component for users to interact with fitness AI assistant
// ABOUTME: Renders chat interface with collapsible conversations panel

import { useState, useEffect, useRef, useCallback } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { ConfirmDialog, TabHeader } from './ui';
import { chatApi, providersApi, coachesApi, oauthApi } from '../services/api';
import { useAuth } from '../hooks/useAuth';
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
import { useUsageStatus } from '../hooks/useUsageStatus';
import ShareChatMessageModal from './social/ShareChatMessageModal';
import { useSuccessToast, useInfoToast } from './ui';
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
import type { CreateCoachRequest, UpdateCoachRequest } from '@pierre/shared-types';

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
  const [newMessage, setNewMessage] = useState('');
  const [isStreaming, setIsStreaming] = useState(false);
  const [streamingContent, setStreamingContent] = useState('');
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [errorCountdown, setErrorCountdown] = useState<number | null>(null);
  const [oauthNotification, setOauthNotification] = useState<OAuthNotification | null>(null);
  const [pendingPrompt, setPendingPrompt] = useState<string | null>(null);
  const [pendingCoachId, setPendingCoachId] = useState<string | null>(null);
  const [showIdeas, setShowIdeas] = useState(false);
  const [connectingProvider, setConnectingProvider] = useState<string | null>(null);
  const [showProviderModal, setShowProviderModal] = useState(false);
  const [pendingCoachAction, setPendingCoachAction] = useState<PendingCoachAction | null>(null);
  const [messageMetadata, setMessageMetadata] = useState<Map<string, MessageMetadata>>(new Map());
  const [messageFeedback, setMessageFeedback] = useState<Map<string, MessageFeedback>>(new Map());
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
  const { data: messagesData, isLoading: messagesLoading } = useQuery<{ messages: Message[] }>({
    queryKey: QUERY_KEYS.chat.messages(selectedConversation),
    queryFn: () => chatApi.getConversationMessages(selectedConversation!),
    enabled: !!selectedConversation,
  });

  // Tier 5.5 — claim verdicts attached to messages in the selected conversation.
  // Refetched alongside messages so a coach reply that triggers verification
  // surfaces its chip without a manual reload.
  const { data: verdictsData } = useQuery({
    queryKey: ['chat', 'verdicts', selectedConversation],
    queryFn: () => chatApi.getConversationVerdicts(selectedConversation!),
    enabled: !!selectedConversation,
  });
  const verdicts: ChatVerdictRow[] = verdictsData?.verdicts ?? [];

  // Drawer state for the Tier 5.5 verdict detail surface.
  const [selectedVerdict, setSelectedVerdict] = useState<ChatVerdictRow | null>(null);

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
        body: JSON.stringify({ content: messageContent }),
      });

      if (!response.ok || !response.body) {
        const errorData = await response.json().catch(() => ({}));
        throw new Error(errorData.error || `HTTP error! status: ${response.status}`);
      }

      let assembled = '';
      let finalData: {
        assistant_message?: { id?: string };
        model?: string;
        execution_time_ms?: number;
        activity_list?: string;
        actions?: MessageActionItem[];
      } | null = null;

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

      const data = finalData ?? {};
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

      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.messages(selectedConversation) });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Failed to send message';
      setErrorMessage(message);
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.messages(selectedConversation) });
    } finally {
      setIsStreaming(false);
      setStreamingContent('');
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
    setConnectingProvider(provider);
    if (selectedConversation) {
      sessionStorage.setItem('pierre_oauth_conversation', selectedConversation);
    }
    try {
      const authUrl = await oauthApi.getAuthorizeUrlForProvider(provider);
      window.open(authUrl, '_blank');
    } catch (error) {
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
        throw new Error(errorData.error || `HTTP error! status: ${response.status}`);
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

  const handleThumbsUp = useCallback((messageId: string) => {
    setMessageFeedback(prev => {
      const newMap = new Map(prev);
      const current = newMap.get(messageId);
      // Toggle: if already up, remove; otherwise set to up
      newMap.set(messageId, current === 'up' ? null : 'up');
      return newMap;
    });
  }, []);

  const handleThumbsDown = useCallback((messageId: string) => {
    setMessageFeedback(prev => {
      const newMap = new Map(prev);
      const current = newMap.get(messageId);
      // Toggle: if already down, remove; otherwise set to down
      newMap.set(messageId, current === 'down' ? null : 'down');
      return newMap;
    });
  }, []);

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
                  className="p-2 rounded-lg text-on-primary bg-pierre-violet hover:bg-pierre-violet-dark transition-colors shadow-ambient hover:shadow-ambient disabled:opacity-50 min-w-[36px] min-h-[36px] flex items-center justify-center"
                  title="New Chat"
                  aria-label="New Chat"
                >
                  <Plus className="w-4 h-4" />
                </button>
              }
            />

            <div className="flex-1 overflow-y-auto">
              <div className="w-full max-w-5xl mx-auto px-6 py-8">
                <div className="text-center mb-8">
                  <h2 className="text-2xl font-semibold text-on-surface mb-2">Ready to analyze your fitness</h2>
                  <p className="text-on-surface-variant text-sm">
                    {hasConnectedProvider
                      ? 'Get personalized insights from your activity data'
                    : 'Or ask a question to Pierre'}
                </p>

                <form
                  onSubmit={(e) => {
                    e.preventDefault();
                    if (newMessage.trim()) {
                      setPendingPrompt(newMessage.trim());
                      createConversation.mutate();
                    }
                  }}
                  className="relative mt-6 max-w-2xl mx-auto"
                >
                  <input
                    type="text"
                    value={newMessage}
                    onChange={(e) => setNewMessage(e.target.value)}
                    placeholder="Message Dravr..."
                    className="w-full rounded-xl border ghost-border bg-surface-container-low text-on-surface placeholder:text-outline pl-4 pr-24 py-3.5 focus:outline-none focus:ring-2 focus:ring-pierre-violet/30 focus:border-pierre-violet text-sm transition-colors"
                    disabled={createConversation.isPending}
                  />
                  <button
                    type="submit"
                    disabled={!newMessage.trim() || createConversation.isPending}
                    className="absolute right-2 top-1/2 -translate-y-1/2 px-4 py-1.5 bg-pierre-violet text-on-primary text-sm font-medium rounded-lg hover:bg-pierre-violet-dark transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-1.5"
                  >
                    {createConversation.isPending ? (
                      <div className="pierre-spinner w-4 h-4 border-white border-t-transparent" />
                    ) : (
                      <>
                        Send
                        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
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

          {/* Conversation Header with Create Coach button */}
          {(messagesData?.messages?.length ?? 0) >= 2 && (
            <div className="border-b ghost-border px-6 py-3 flex items-center justify-end">
              <button
                onClick={() => setShowCreateCoachFromConversation(true)}
                disabled={isStreaming}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium text-pierre-violet bg-pierre-violet/10 hover:bg-pierre-violet/20 rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                title="Create a coach based on this conversation"
              >
                <Sparkles className="w-3.5 h-3.5" />
                Create Coach
              </button>
            </div>
          )}
          <div className="flex-1 overflow-y-auto min-h-0">
            <div className="max-w-3xl mx-auto py-6 px-6">
              <MessageList
                messages={messagesData?.messages || []}
                messageMetadata={messageMetadata}
                messageFeedback={messageFeedback}
                activityLists={activityLists}
                messageActions={messageActions}
                insightMessageIds={new Set<string>()}
                verdicts={verdicts}
                isLoading={messagesLoading}
                isStreaming={isStreaming}
                streamingContent={streamingContent}
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
