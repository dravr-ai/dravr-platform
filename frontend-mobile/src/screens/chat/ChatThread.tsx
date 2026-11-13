// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One conversation's transcript and composer — the part of a thread every chat surface on the phone shares
// ABOUTME: The chat screen and an activity's view both mount it; in a coach's one-athlete room it also routes the private lane

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Alert, Keyboard, Text, TextInput, TouchableOpacity } from 'react-native';
import * as Clipboard from 'expo-clipboard';
import { useTranslation } from '@pierre/i18n';
import {
  coachComposerPlaceholder,
  composeRoomThread,
  PLAN_THREAD_KEYS,
  PRIVATE_LANE_KEYS,
  privateLaneComposerText,
  threadCarriesDraft,
  transcriptBlocks,
  trustedActionUrl,
  verdictSupportReference,
} from '@pierre/chat-utils';
import { cardForSlug, connectPlan, noticeRequired, sciotteTargetForBackend } from '@pierre/shared-constants';
import { privateSendFailureText, type PlanThreadDraft, type UsePrivateLaneResult } from '@pierre/ui-logic';
import type {
  ChatMessageAction,
  ClaimVerdict,
  Message,
  MessageAudience,
  MessageOrigin,
  SciotteTarget,
  TurnSendOptions,
  UseCaseStarter,
} from '@pierre/shared-types';

import { Sheet } from '../../components/ui';
import { IntervalsIcuLinkModal } from '../../components/IntervalsIcuLinkModal';
import { SciotteLoginModal } from '../../components/SciotteLoginModal';
import { useSignInModal } from '../../hooks/useSignInModal';
import { OAuthCredentialsSection } from '../../components/OAuthCredentialsSection';
import { ProviderNoticeSheet } from '../../components/ProviderNotice';
import { openExternal } from '../../utils/openExternal';
import { ChatInputBar, type ComposerAudience, type ComposerQuote } from './ChatInputBar';
import { MessageList, type MessageListPrivateLane } from './MessageList';
import { UsageWarningBanner } from './UsageWarningBanner';
import { VerdictSheet, type VerdictSource } from './VerdictSheet';
import PlanThreadOpenDraft from './PlanThreadOpenDraft';
import { useChatVoiceInput } from './useChatVoiceInput';
import type { useMessages } from './useMessages';
import type { useProviderStatus } from './useProviderStatus';
import type { useUsageStatus } from './useUsageStatus';

export interface ChatThreadProps {
  /** The conversation on screen, or `null` before its first turn. */
  conversationId: string | null;
  /** The transcript's state, owned by the host so it can read and reload it. */
  messagesHook: ReturnType<typeof useMessages>;
  usageStatus: ReturnType<typeof useUsageStatus>;
  providerStatus: ReturnType<typeof useProviderStatus>;
  /** The host is still resolving which conversation this is. */
  isLoading: boolean;
  /** The composer's text, held by the host so a navigation can draft into it. */
  inputText: string;
  onChangeInputText: (text: string) => void;
  inputRef: React.RefObject<TextInput | null>;
  /**
   * Send one line as the next turn. The host resolves the conversation —
   * opening one when there is none — so every turn, typed or pressed, goes
   * through its one implementation.
   */
  sendText: (text: string, options?: TurnSendOptions) => Promise<void>;
  /**
   * How the composer's text came to be there: `draft` while it holds what a
   * draft affordance put in (carnet#828). The host knows; absent means typed.
   */
  composerOrigin?: MessageOrigin;
  /** Drawn above the transcript, scrolling with it. */
  header?: React.ReactElement;
  /**
   * The use-case starters the empty thread offers, ranked for the athlete. A
   * host that leaves them out draws the empty thread without them.
   */
  starters?: UseCaseStarter[];
  /**
   * The athlete a coach's plan thread is about. Its empty state is the
   * coach's planning space, not the athlete's own "ask anything".
   */
  planSubjectName?: string | null;
  /**
   * The group's name, when the viewer is its human coach in its room. Its
   * empty state and composer say who reads what the coach writes there.
   */
  coachRoomName?: string | null;
  /**
   * The host has not yet read whose thread this is — the routed conversation,
   * or its group's coach. The composer shows no placeholder until it has, so
   * a coach's thread never opens on the athlete's words.
   */
  composerPending?: boolean;
  /**
   * The plan thread's draft read, which the host makes in a coach's plan
   * thread: the open draft to draw when no card in the thread shows it, and
   * whether the coach holds none, when the empty thread offers its start.
   */
  planDraftRead?: PlanThreadDraft;
  /** How the transcript follows its content; the list's own scroll-to-end when absent. */
  onScrollToBottom?: () => void;
  /**
   * What the conversation is called, as its header shows it. The verdict
   * sheet names it beside the reply a claim came from; a host that leaves it
   * out gets no conversation section there.
   */
  conversationTitle?: string;
  /**
   * A coach's private lane in the one-athlete room on screen (carnet#1249) —
   * the host's one `usePrivateLane` — with the room's agent as it is named.
   * Absent in every other thread; a lane that is not `available` draws nothing.
   */
  privateLane?: { lane: UsePrivateLaneResult; agentName: string } | null;
  /** The host's one-time offer at the end of the transcript, in the agent's voice. */
  offer?: React.ReactNode;
}

/** No plan thread, or its draft read still out. */
const NO_PLAN_DRAFT: PlanThreadDraft = { open: null, noDraft: false };

/** Nothing private to draw. */
const NO_PRIVATE_ROWS: readonly Message[] = [];

/**
 * A thread: the transcript, the progress line, the usage banner and the
 * composer, with every handler a reply's controls need.
 *
 * A reply can carry links, postback buttons, a reconnect prompt and claim
 * verdicts; each is handled here once, so a thread opened from an activity's
 * view behaves exactly as one opened from the chat tab.
 */
export function ChatThread({
  conversationId,
  messagesHook,
  usageStatus,
  providerStatus,
  isLoading,
  inputText,
  onChangeInputText,
  inputRef,
  sendText,
  composerOrigin,
  header,
  starters,
  planSubjectName = null,
  coachRoomName = null,
  composerPending = false,
  planDraftRead = NO_PLAN_DRAFT,
  onScrollToBottom,
  conversationTitle,
  privateLane = null,
  offer = null,
}: ChatThreadProps) {
  const { t } = useTranslation();
  // The lane while it exists: the viewer is the group's coach and the room
  // holds one athlete. Everything private below hangs off this one value.
  const lane = privateLane?.lane.available && privateLane.lane.athlete ? privateLane.lane : null;
  const agentName = privateLane?.agentName ?? '';
  const athleteName = lane?.athlete?.display_name ?? '';
  const sendsPrivately = lane?.audience === 'private';
  // What the last private send came to, when it failed: said over the composer.
  const [privateFailure, setPrivateFailure] = useState<string | null>(null);
  // The composer's text as of the last render, so a refused private send puts
  // its words back only into a field the coach has not typed in since.
  const inputTextRef = useRef(inputText);
  useEffect(() => {
    inputTextRef.current = inputText;
  }, [inputText]);
  // The message whose verdicts the sheet shows, or `null` while it is closed.
  const [verdictMessageId, setVerdictMessageId] = useState<string | null>(null);
  // The provider whose notice is on screen before a reconnect starts its
  // OAuth flow (WHOOP, until the account accepts its owner authorization).
  const [noticeFor, setNoticeFor] = useState<string | null>(null);
  // The Intervals.icu sheet a reconnect opened, and whether it leads with the
  // authorization (`intervalsConnectPath` answered `'choose'`) or is the key
  // form alone.
  const [intervalsSheet, setIntervalsSheet] = useState<{ offersAuthorize: boolean } | null>(null);
  // The provider sign-in a reply's reconnect opens for a card that signs in
  // (COROS, TrainingPeaks, Strava or Garmin with no seat).
  const signIn = useSignInModal();
  const openSignIn = signIn.open;
  const closeSignIn = signIn.close;

  // Opening the keyboard shortens the visible list. `onContentSizeChange` only
  // fires when the CONTENT changes, so tapping into the composer on an existing
  // thread left the newest messages above the fold with nothing to bring them
  // back.
  const scrollToBottom = onScrollToBottom ?? messagesHook.scrollToBottom;
  useEffect(() => {
    const shown = Keyboard.addListener('keyboardDidShow', scrollToBottom);
    return () => shown.remove();
  }, [scrollToBottom]);

  // Voice input with chat-specific error handling
  const voiceInput = useChatVoiceInput(
    (text) => onChangeInputText(text),
    onChangeInputText
  );

  // A turn's pre-turn quota check reports its counters as a `notice` reply
  // block. Hand it to the banner, which is the one place a cap is stated.
  const { quotaNotice } = messagesHook;
  const { applyNotice } = usageStatus;
  useEffect(() => {
    if (quotaNotice) applyNotice(quotaNotice);
  }, [quotaNotice, applyNotice]);

  // Whether the composer holds a draft this thread's own affordances put there
  // — "back up this claim" — so its send is reported as one (carnet#828). A
  // draft the athlete erased entirely is no longer one.
  const [threadDrafted, setThreadDrafted] = useState(false);

  // A starter tapped in the empty thread is sent at once: its postback, under
  // its label. The host opens the thread for it with the athlete speaking
  // first, so the agent posts no welcome offering the same starters again
  // over the answer (carnet#829).
  const handleStarter = useCallback(
    (starter: UseCaseStarter) => {
      void sendText(starter.postback, { display: starter.label });
    },
    [sendText],
  );
  useEffect(() => {
    if (inputText === '') setThreadDrafted(false);
  }, [inputText]);

  /**
   * Open a link a reply carries. A reply is model-authored, so it may only
   * send the athlete to a web page: anything but http(s) is refused here,
   * before `openExternal` (which judges no URL) hands it to the device and
   * reports a device that cannot open it. An action button's link is
   * vetted further, by host, in `handleActionClick` before it reaches here.
   */
  const handleOpenUrl = useCallback(async (url: string) => {
    let parsedUrl: URL;
    try {
      parsedUrl = new URL(url);
    } catch {
      console.error('Invalid URL:', url);
      Alert.alert(t('app.linkErrorTitle'), t('app.linkInvalidFormat'));
      return;
    }

    const scheme = parsedUrl.protocol.toLowerCase();
    if (scheme !== 'http:' && scheme !== 'https:') {
      console.warn('Blocked non-HTTP URL scheme:', scheme);
      Alert.alert(t('app.linkBlockedTitle'), t('app.linkBlockedBody'));
      return;
    }

    await openExternal(url, t);
  }, [t]);

  /**
   * One of the coach's private turns, to their work thread about the room's
   * athlete (carnet#1249). The lane carries the quote and writes the rows;
   * the thread draws the turn live. A send the server refused before any turn
   * ran stored nothing, so typed words go back in the composer when the coach
   * has not typed since; every failure is said over the composer.
   */
  const sendPrivate = useCallback(
    async (text: string, options: TurnSendOptions, restoreOnRefusal: boolean) => {
      if (!lane) return;
      setPrivateFailure(null);
      const outcome = await messagesHook.sendPrivateTurn(lane.send, text, options);
      if (outcome.kind !== 'failed') return;
      if (outcome.refused && restoreOnRefusal && inputTextRef.current.trim() === '') {
        onChangeInputText(text);
      }
      setPrivateFailure(privateSendFailureText(outcome.error, { t, athleteName }));
    },
    [lane, messagesHook, onChangeInputText, t, athleteName],
  );

  const handleSendMessage = useCallback(async () => {
    const messageText = inputText.trim();
    if (!messageText) return;
    onChangeInputText('');
    setPrivateFailure(null);
    const options: TurnSendOptions = {
      origin: composerOrigin ?? (threadDrafted ? 'draft' : undefined),
    };
    // The switch decides where the line goes: "Only <agent>" reaches the
    // coach's work thread and never the room.
    if (sendsPrivately) {
      await sendPrivate(messageText, options, true);
      return;
    }
    await sendText(messageText, options);
  }, [inputText, onChangeInputText, sendText, composerOrigin, threadDrafted, sendsPrivately, sendPrivate]);

  /**
   * Press handler for a control the reply's `actions` block carried.
   *
   * A `postback` sends its `value` as the next turn, so the press flows
   * through the same dispatch pipeline a typed command would. A command's
   * value is the line the athlete would type, so it is what their bubble
   * shows; any other value is a suggestion's postback, which the server
   * resolves into its words, so the bubble shows the label until the turn
   * echoes them (carnet#828). A `url` opens
   * its `value` in the system browser — but only after `trustedActionUrl`
   * vouches for the host: the value reaches the client inside a
   * model-adjacent reply, so an unvouched address is an open redirect wearing
   * a button. A refused URL opens nothing.
   *
   * A control on a coach's private reply answers in private: posted to the
   * room, it would put the coach's draft in front of the athlete. With no
   * lane to answer in, it sends nothing.
   */
  const handleActionClick = useCallback(
    async (action: ChatMessageAction, audience: MessageAudience = 'room') => {
      if (action.action_type === 'url') {
        const target = trustedActionUrl(action.value, [
          process.env.EXPO_PUBLIC_API_URL ?? '',
        ]);
        if (target) await handleOpenUrl(target);
        return;
      }
      const options = action.value.startsWith('/') ? undefined : { display: action.label };
      if (audience === 'private') {
        await sendPrivate(action.value, options ?? {}, false);
        return;
      }
      await sendText(action.value, options);
    },
    [handleOpenUrl, sendText, sendPrivate],
  );

  // Retry, and the feedback handlers, name the open conversation so the hook
  // can re-send or persist thumbs up/down and an optional reason against it.
  const handleRetryMessage = useCallback(async (messageId: string) => {
    if (!conversationId) return;
    await messagesHook.retryMessage(messageId, conversationId);
  }, [messagesHook, conversationId]);

  const handleThumbsUp = useCallback((messageId: string) => {
    if (!conversationId) return;
    void messagesHook.handleThumbsUp(messageId, conversationId);
  }, [messagesHook, conversationId]);

  const handleThumbsDown = useCallback((messageId: string) => {
    if (!conversationId) return;
    void messagesHook.handleThumbsDown(messageId, conversationId);
  }, [messagesHook, conversationId]);

  const handleSubmitFeedbackReason = useCallback((messageId: string, comment: string) => {
    if (!conversationId) return;
    void messagesHook.submitFeedbackReason(messageId, conversationId, comment);
  }, [messagesHook, conversationId]);

  // The rows are written right after the reply row, so a chip that landed
  // before the read did opens the sheet on a re-read rather than on nothing.
  const { refreshVerdicts, verdicts, verdictsLoading } = messagesHook;
  const sheetVerdicts = useMemo(
    () => (verdictMessageId ? verdicts.filter((v) => v.message_id === verdictMessageId) : []),
    [verdicts, verdictMessageId],
  );
  const handleShowVerdict = useCallback((rows: ClaimVerdict[], messageId: string) => {
    setVerdictMessageId(messageId);
    if (rows.length === 0 && conversationId) void refreshVerdicts(conversationId);
  }, [refreshVerdicts, conversationId]);

  const handleAskAboutClaim = useCallback((verdict: ClaimVerdict) => {
    setThreadDrafted(true);
    onChangeInputText(t('app.backUpClaim', { claim: verdict.claim_text }));
    setVerdictMessageId(null);
  }, [onChangeInputText, t]);

  // The reply the sheet's verdicts were drawn from. The sheet opens from a
  // chip under that reply, so it is in the transcript on screen.
  const verdictSource = useMemo<VerdictSource | undefined>(() => {
    if (!conversationTitle || !verdictMessageId) return undefined;
    const reply = messagesHook.messages.find((m) => m.id === verdictMessageId);
    return reply ? { title: conversationTitle, content: reply.content, createdAt: reply.created_at } : undefined;
  }, [conversationTitle, verdictMessageId, messagesHook.messages]);

  // What support needs to find a verdict. The sheet does not print these ids;
  // it hands them over from its actions menu.
  const handleCopyVerdictReference = useCallback(async (verdict: ClaimVerdict) => {
    try {
      await Clipboard.setStringAsync(verdictSupportReference(verdict));
      Alert.alert(t('app.copiedTitle'));
    } catch {
      Alert.alert(t('app.copyFailed'));
    }
  }, [t]);

  /**
   * Reconnect the provider a reply names (`garmin`, `strava`, `sciotte_coros`,
   * `whoop`…) the way its card's `connectPlan` says.
   *
   * An authorization runs through `WebBrowser.openAuthSessionAsync`, which
   * presents a sheet over the app that hands the callback back to it, named
   * as the authorize route takes it — never a card's `sciotte_*` key.
   * Opening the reply's URL with the generic opener instead sends the athlete
   * to Safari, where the callback has nowhere to return to. A card that signs
   * in opens the sign-in window here.
   */
  const handleConnectProvider = useCallback(async (provider: string) => {
    const cards = providerStatus.connectedProviders;
    const card = cardForSlug(provider, cards) ?? cards.find((p) => p.provider === provider);
    if (!card) {
      // No card read yet: a sign-in backend still signs in, anything else
      // is the authorization of the slug itself.
      const target = sciotteTargetForBackend(provider);
      if (target) {
        openSignIn(target, undefined);
        return;
      }
      await providerStatus.handleConnectProvider(provider);
      return;
    }
    const primary = connectPlan(card).primary;
    // Intervals.icu reconnects the way its card was linked, and never through
    // an own-app setup: a key-linked card (a coach's — only the key reads the
    // athletes they coach) reopens the key form, an authorized card goes back
    // to the authorization, and with no client to authorize under the key
    // form is the only way in.
    if (primary.kind === 'intervals') {
      if (primary.path !== 'authorize') {
        setIntervalsSheet({ offersAuthorize: primary.path === 'choose' });
        return;
      }
      await providerStatus.handleConnectProvider(card.provider);
      return;
    }
    if (primary.kind === 'sign_in') {
      // The card's own plan: a sign-in-first card's sign-in that cannot
      // complete falls back to its OAuth (carnet#1477).
      openSignIn(primary.target, card, false, true);
      return;
    }
    // No app of the server's can authorize the provider for this athlete:
    // the credentials sheet asks for their own before any notice.
    if (card.own_app_required) {
      providerStatus.setNeedsCredentialsProvider(card.provider);
      return;
    }
    // A provider whose notice the account has not accepted (WHOOP's owner
    // authorization) states it first; its Continue starts the flow.
    if (noticeRequired(card.provider, card.consent_required)) {
      setNoticeFor(card.provider);
      return;
    }
    await providerStatus.handleConnectProvider(primary.kind === 'oauth' ? primary.provider : card.provider);
  }, [providerStatus, openSignIn]);

  // A sign-in that could not complete hands over to its provider's OAuth
  // (carnet#1477): the window closes and the authorization starts instead.
  const fallBackToOAuth = useCallback(
    (provider: SciotteTarget) => () => {
      closeSignIn();
      void providerStatus.handleConnectProvider(provider);
    },
    [closeSignIn, providerStatus],
  );

  // An app saved in the credentials sheet changes the status the gate above
  // reads (`own_app_required`), so closing the sheet re-reads it: the next
  // connect then starts the flow instead of reopening the sheet.
  const closeCredentials = useCallback(() => {
    providerStatus.setNeedsCredentialsProvider(null);
    void providerStatus.loadProviderStatus();
  }, [providerStatus]);

  // The room with the coach's private rows woven in, in time order. They are
  // drawn here and never join the thread's own rows: a retry, the read marker
  // and the lost-turn note act on those, and a private line taken for one of
  // them would be said to the room.
  const privateRows = lane?.rows ?? NO_PRIVATE_ROWS;
  const threadMessages = useMemo(
    () =>
      privateRows.length > 0 ? composeRoomThread(messagesHook.messages, [], privateRows) : messagesHook.messages,
    [messagesHook.messages, privateRows],
  );

  // A private reply the server is still writing that no send here streams —
  // the app reopened mid-turn — runs the indicator and the stop button too.
  const privateAwaiting = Boolean(lane?.awaitingReply) && !messagesHook.isSending;
  const turnRunning = messagesHook.turnInFlight || privateAwaiting;
  const laneConversationId = lane?.conversationId ?? null;
  const { privateTurnInFlight, turnInFlight, stopTurn, stopPrivateTurn } = messagesHook;
  const handleStopTurn = useCallback(() => {
    const stopsPrivate = privateTurnInFlight || (!turnInFlight && privateAwaiting);
    if (stopsPrivate) {
      // Before the lane has opened the work thread there is no turn to stop yet.
      if (laneConversationId) void stopPrivateTurn(laneConversationId);
      return;
    }
    void stopTurn();
  }, [privateTurnInFlight, turnInFlight, privateAwaiting, laneConversationId, stopPrivateTurn, stopTurn]);

  const handleAdjustFrom = useCallback(
    (row: Message) => {
      if (lane?.adjustFrom(row)) {
        setPrivateFailure(null);
        inputRef.current?.focus();
      }
    },
    [lane, inputRef],
  );

  const listLane: MessageListPrivateLane | undefined = lane
    ? {
        conversationId: lane.conversationId,
        agentName,
        canAdjustFrom: lane.canAdjustFrom,
        onAdjustFrom: handleAdjustFrom,
        turnLive: privateTurnInFlight,
      }
    : undefined;
  const composerAudience: ComposerAudience | undefined = lane
    ? {
        value: lane.audience,
        text: privateLaneComposerText(lane.audience, { agent: agentName, athlete: athleteName }, t),
        onToggle: () => {
          setPrivateFailure(null);
          lane.toggleAudience();
        },
      }
    : undefined;
  // A coach's own threads say whom the line goes to; an athlete's composer
  // keeps the app's own words; one not yet known says nothing.
  const coachPlaceholder =
    coachComposerPlaceholder(t, { planSubjectName, coachRoomName, pending: composerPending }) ?? undefined;

  // The coach's open draft on the plan thread's athlete, drawn whenever no
  // card in the thread shows it: "Review the draft" lands on a thread whose
  // saving reply may be elsewhere, or not there at all. A row read back
  // carries no block list on the wire: its blocks are decoded from the row,
  // as its bubble decodes them.
  const openDraftId = planDraftRead.open?.draft.draft_id ?? null;
  const { messageBlocks } = messagesHook;
  const draftCarried = useMemo(
    () =>
      openDraftId !== null &&
      threadCarriesDraft(
        threadMessages.map((message) => messageBlocks[message.id] ?? transcriptBlocks(message)),
        openDraftId,
      ),
    [openDraftId, threadMessages, messageBlocks],
  );
  const openDraft = draftCarried ? null : planDraftRead.open;
  const planDraft =
    openDraft !== null && conversationId !== null ? (
      <PlanThreadOpenDraft conversationId={conversationId} open={openDraft} />
    ) : null;
  // The empty plan thread's one-tap start while the coach holds no draft:
  // the plan request, sent in the coach's words under its label.
  const planStarter =
    planSubjectName !== null && planDraftRead.noDraft
      ? {
          label: t(PLAN_THREAD_KEYS.starterLabel, { name: planSubjectName }),
          onPress: () => void sendText(t(PLAN_THREAD_KEYS.starterMessage, { name: planSubjectName })),
          disabled: turnRunning || usageStatus.sendDisabled,
        }
      : null;
  // The quote rides a private send only, so the chip shows only while the
  // next send is private: the composer shows what the next send carries.
  const composerQuote: ComposerQuote | null = lane?.quote && sendsPrivately
    ? {
        label: t(PRIVATE_LANE_KEYS.quoteChip, { name: lane.quote.author_display_name }),
        removeLabel: t(PRIVATE_LANE_KEYS.quoteRemove),
        onRemove: lane.removeQuote,
      }
    : null;

  return (
    <>
      <VerdictSheet
        visible={verdictMessageId !== null}
        verdicts={sheetVerdicts}
        loading={verdictsLoading && sheetVerdicts.length === 0}
        onClose={() => setVerdictMessageId(null)}
        onAskAboutClaim={handleAskAboutClaim}
        source={verdictSource}
        onCopyReference={(verdict) => void handleCopyVerdictReference(verdict)}
      />

      <MessageList
        conversationId={conversationId}
        messages={threadMessages}
        isLoading={isLoading}
        isSending={turnRunning}
        liveTrail={messagesHook.liveTrail}
        streamingContent={messagesHook.streamingContent}
        awaitingSince={messagesHook.awaitingSince}
        turnTrails={messagesHook.turnTrails}
        messageFeedback={messagesHook.messageFeedback}
        messageFeedbackComment={messagesHook.messageFeedbackComment}
        messageBlocks={messagesHook.messageBlocks}
        verdicts={messagesHook.verdicts}
        flatListRef={messagesHook.flatListRef}
        onScrollToBottom={scrollToBottom}
        onThumbsUp={handleThumbsUp}
        onThumbsDown={handleThumbsDown}
        onSubmitFeedbackReason={handleSubmitFeedbackReason}
        onRetryMessage={handleRetryMessage}
        onOpenUrl={handleOpenUrl}
        onReconnectProvider={handleConnectProvider}
        onActionClick={handleActionClick}
        onShowVerdict={handleShowVerdict}
        header={header}
        starters={starters}
        planSubjectName={planSubjectName}
        coachRoomName={coachRoomName}
        planDraft={planDraft}
        offer={offer}
        planStarter={planStarter}
        onStarter={handleStarter}
        hasEarlier={messagesHook.hasEarlier}
        loadingEarlier={messagesHook.loadingEarlier}
        onReachStart={() => void messagesHook.loadEarlier()}
        privateLane={listLane}
      />

      <UsageWarningBanner level={usageStatus.level} message={usageStatus.message} />

      {privateFailure ? (
        <Text
          testID="private-send-failed"
          accessibilityRole="alert"
          accessibilityLiveRegion="polite"
          className="px-4 pt-2 text-sm text-error"
        >
          {privateFailure}
        </Text>
      ) : null}

      <ChatInputBar
        inputText={inputText}
        partialTranscript={voiceInput.partialTranscript}
        isListening={voiceInput.isListening}
        isSending={turnRunning}
        isStopping={messagesHook.isStopping}
        onStopTurn={handleStopTurn}
        disabled={usageStatus.sendDisabled}
        voiceAvailable={voiceInput.isAvailable}
        inputRef={inputRef}
        onChangeText={onChangeInputText}
        onVoicePress={voiceInput.handleVoicePress}
        onSendMessage={handleSendMessage}
        audience={composerAudience}
        placeholder={coachPlaceholder}
        quote={composerQuote}
      />

      {/* A reply asked for provider credentials the app does not hold yet. */}
      <ProviderNoticeSheet
        provider={noticeFor}
        onCancel={() => setNoticeFor(null)}
        onAccept={() => {
          const accepted = noticeFor;
          setNoticeFor(null);
          if (accepted) void providerStatus.handleConnectProvider(accepted, undefined, true);
        }}
      />

      <IntervalsIcuLinkModal
        visible={intervalsSheet !== null}
        onClose={() => setIntervalsSheet(null)}
        onAuthorize={
          intervalsSheet?.offersAuthorize
            ? () => {
                setIntervalsSheet(null);
                void providerStatus.handleConnectProvider('intervals_icu');
              }
            : undefined
        }
        onConnected={() => {
          setIntervalsSheet(null);
          void providerStatus.refreshAfterLink();
        }}
      />

      {/* Mounted only while a reply's reconnect signs in, so a thread that
          never asks carries none of the sign-in's state. */}
      {signIn.target !== null && (
        <SciotteLoginModal
          visible
          onClose={signIn.close}
          onConnected={() => {
            signIn.close();
            void providerStatus.refreshAfterLink();
          }}
          target={signIn.target}
          consentRequired={signIn.consentRequired}
          onOAuthRequired={() => void providerStatus.refreshAfterLink()}
          onFallBackToOAuth={signIn.fallback ? fallBackToOAuth(signIn.fallback) : undefined}
        />
      )}

      <Sheet
        visible={providerStatus.needsCredentialsProvider !== null}
        onClose={closeCredentials}
        testID="oauth-credentials-sheet"
        flush
      >
        <OAuthCredentialsSection providers={providerStatus.connectedProviders} />
        <TouchableOpacity
          className="mt-4 py-3 items-center"
          onPress={closeCredentials}
        >
          <Text className="text-base text-text-tertiary">{t('common.close')}</Text>
        </TouchableOpacity>
      </Sheet>
    </>
  );
}
