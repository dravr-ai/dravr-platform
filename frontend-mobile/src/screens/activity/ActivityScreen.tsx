// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One activity's own screen, opened from Home — its map on top, its figures, splits and laps, then the chat about it
// ABOUTME: The chat screen's own thread and composer under the activity, one conversation per activity, reopened on a return visit

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, Text, TextInput, View } from 'react-native';
import { KeyboardAvoidingView } from 'react-native-keyboard-controller';
import { useHeaderHeight } from 'expo-router/react-navigation';
import { Stack, useLocalSearchParams } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import type { ActivityDetailResponse } from '@pierre/shared-types';
import {
  ACTIVITY_PROMPTS,
  activityFigures,
  activityFirstLine,
  describeApiError,
  lapsTable,
  splitsTable,
  type SegmentTable,
} from '@pierre/ui-logic';
import { EmptyState, Section } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useActivityConversation } from '../../hooks/useActivityConversation';
import { useActivityDetail } from '../../hooks/useActivityDetail';
import { trackMobile } from '../../services/analytics';
import { ChatThread } from '../chat/ChatThread';
import { useConversations } from '../chat/useConversations';
import { useMessages } from '../chat/useMessages';
import { useProviderStatus } from '../chat/useProviderStatus';
import { useUsageStatus } from '../chat/useUsageStatus';
import { ActivityMap } from '../home/ActivityMap';
import { activityNaming, instantShortDate, sportLabel } from '../home/homeFormat';

/** The figures, label over value, two to a row. */
function Figures({ detail }: { detail: ActivityDetailResponse }) {
  const { t, language } = useTranslation();
  return (
    <View className="flex-row flex-wrap px-4" testID="activity-figures">
      {activityFigures(t, detail, language).map((figure) => (
        <View key={figure.id} className="w-1/2 py-2" testID={`activity-figure-${figure.id}`}>
          <Text className="text-sm text-text-secondary">{t(figure.labelKey)}</Text>
          <Text className="font-mono text-base tabular-nums text-text-primary">{figure.value}</Text>
        </View>
      ))}
    </View>
  );
}

/** A splits or laps table; a column no row fills is not drawn. */
function Segments({ title, table, testID }: { title: string; table: SegmentTable; testID: string }) {
  const { t } = useTranslation();
  // Six columns share a phone's width: every cell pads its own left edge so
  // neighbouring figures never run together, at the caption size.
  const cell = 'flex-1 pl-1 text-right font-mono text-xs tabular-nums text-text-primary';
  const head = 'flex-1 pl-1 text-right text-xs text-text-secondary';
  return (
    <Section title={title} testID={testID}>
      <View className="px-4">
        <View className="flex-row border-b border-border-faint py-2">
          <Text className={`${head} max-w-6`}>#</Text>
          <Text className={head}>{t('home.activity.figure.distance')}</Text>
          <Text className={head}>{t('home.activity.column.time')}</Text>
          {table.hasSpeed && <Text className={head}>{t(table.speedLabelKey)}</Text>}
          {table.hasHeartRate && <Text className={head}>{t('home.activity.column.heartRate')}</Text>}
          {table.hasElevation && <Text className={head}>{t('home.activity.column.elevation')}</Text>}
        </View>
        {table.rows.map((row) => (
          <View key={row.index} className="flex-row border-b border-border-faint py-2" testID={`${testID}-row-${row.index}`}>
            <Text className={`${cell} max-w-6`}>{row.index}</Text>
            <Text className={cell}>{row.distance}</Text>
            <Text className={cell}>{row.time}</Text>
            {table.hasSpeed && <Text className={cell}>{row.speed ?? ''}</Text>}
            {table.hasHeartRate && <Text className={cell}>{row.heartRate ?? ''}</Text>}
            {table.hasElevation && <Text className={cell}>{row.elevation ?? ''}</Text>}
          </View>
        ))}
      </View>
    </Section>
  );
}

/**
 * The questions about the activity, over its thread: one chip per suggested
 * question, each sent word for word and naming the activity by title and day.
 */
function Prompts({
  detail,
  disabled,
  error,
  onSend,
}: {
  detail: ActivityDetailResponse;
  disabled: boolean;
  error: string | null;
  onSend: (text: string) => void;
}) {
  const { t, language } = useTranslation();
  const naming = activityNaming(t, detail.activity, language);
  return (
    <Section title={t('home.activity.chatHeading')} testID="activity-chat">
      <View className="flex-row flex-wrap gap-2 px-4" testID="activity-prompts">
        {ACTIVITY_PROMPTS.map((prompt) => (
          <Pressable
            key={prompt.id}
            accessibilityRole="button"
            accessibilityState={{ disabled }}
            disabled={disabled}
            onPress={() => onSend(t(prompt.textKey, naming))}
            className={`min-h-11 justify-center rounded-lg border border-border-strong px-4 ${disabled ? 'opacity-50' : ''}`}
            testID={`activity-prompt-${prompt.id}`}
          >
            <Text className="text-sm text-text-primary">{t(prompt.labelKey)}</Text>
          </Pressable>
        ))}
      </View>
      {error !== null && (
        <Text className="mt-2 px-4 text-sm text-error" testID="activity-chat-error">
          {error}
        </Text>
      )}
    </Section>
  );
}

/** Everything above the thread: when, the map, the figures, splits and laps, the questions. */
function ActivityHeader({
  detail,
  sending,
  error,
  onSend,
}: {
  detail: ActivityDetailResponse;
  sending: boolean;
  error: string | null;
  onSend: (text: string) => void;
}) {
  const { t, language } = useTranslation();
  const sport = sportLabel(t, detail.activity.sport_type);
  const splits = splitsTable(detail, language);
  const laps = lapsTable(detail, language);
  // The thread's list pads its rows; the header runs edge to edge like the
  // screen it replaced, each block keeping its own gutter.
  return (
    <View className="-mx-4 mb-6 gap-8" testID="activity-scroll">
      <View>
        <Text className="px-4 pb-2 text-sm text-text-secondary" testID="activity-when">
          <Text className="font-mono tabular-nums">{instantShortDate(detail.activity.start_date, language)}</Text>
          {' · '}
          {sport}
          {/* The attribution a Garmin-recorded activity carries, as served (carnet#521). */}
          {detail.activity.attribution && (
            <Text testID="activity-attribution">{` · ${detail.activity.attribution}`}</Text>
          )}
        </Text>
        <ActivityMap activity={detail.activity} testIDPrefix="activity" />
      </View>
      <Figures detail={detail} />
      {splits && <Segments title={t('home.activity.splits')} table={splits} testID="activity-splits" />}
      {laps && <Segments title={t('home.activity.laps')} table={laps} testID="activity-laps" />}
      <Prompts detail={detail} disabled={sending} error={error} onSend={onSend} />
    </View>
  );
}

/**
 * The activity and the conversation about it, in one column: the activity on
 * top, the thread under it, the composer at the foot.
 *
 * The thread is the chat screen's own — the same transcript, the same
 * composer, every reply control — mounted under the activity instead of on a
 * screen of its own, so asking never takes the athlete away from the figures
 * the answer is about. One conversation per activity, linked to it on the
 * server: the first question opens it, and later questions, return visits
 * and the same activity opened on another device reuse it.
 */
function ActivityThread({
  provider,
  activityId,
  detail,
}: {
  provider: string;
  activityId: string;
  detail: ActivityDetailResponse;
}) {
  const { t, language } = useTranslation();
  const headerHeight = useHeaderHeight();
  const { conversationId, setConversationId } = useActivityConversation(provider, activityId, detail);
  const conversations = useConversations();
  const messagesHook = useMessages();
  const usageStatus = useUsageStatus();
  const providerStatus = useProviderStatus();
  const inputRef = useRef<TextInput>(null);
  const [inputText, setInputText] = useState('');
  const [createError, setCreateError] = useState<string | null>(null);
  // The thread this screen just created: its first turn is already painting
  // its rows, and a read of the empty thread must not race them.
  const justCreatedRef = useRef<string | null>(null);
  // The thread follows its newest row only once the athlete has asked
  // something on this visit; before that the screen opens on the activity.
  const followRef = useRef(false);

  const { loadMessages, clearMessages, scrollToBottom } = messagesHook;
  const { loadProviderStatus } = providerStatus;
  useEffect(() => {
    loadProviderStatus();
  }, [loadProviderStatus]);

  // The thread is read when it changes — on arrival with one already opened,
  // after a `/reset` moved it — and only then. `loadMessages` is a new
  // function whenever the transcript grows, so it is read through a ref: an
  // effect keyed on it read the thread again after every turn and painted
  // the server's copy over rows still on their way.
  const threadReads = useRef({ loadMessages, clearMessages });
  threadReads.current = { loadMessages, clearMessages };
  useEffect(() => {
    if (conversationId === null) {
      threadReads.current.clearMessages();
      return;
    }
    if (justCreatedRef.current === conversationId) {
      justCreatedRef.current = null;
      return;
    }
    void threadReads.current.loadMessages(conversationId);
  }, [conversationId]);

  const followThread = useCallback(() => {
    if (followRef.current) scrollToBottom();
  }, [scrollToBottom]);

  const naming = activityNaming(t, detail.activity, language);

  /**
   * Send one line as the next turn of this activity's thread, opening the
   * thread on the first. A typed first line goes out in the sentence that
   * names the activity, since the new thread has no other word on which
   * activity it is about; a suggested question already names it.
   */
  const send = useCallback(async (text: string, typed: boolean) => {
    const trimmed = text.trim();
    if (!trimmed || messagesHook.isSending) return;
    followRef.current = true;
    setCreateError(null);

    let target = conversationId;
    let line = trimmed;
    if (target === null) {
      if (typed) line = activityFirstLine(t, naming, trimmed);
      try {
        const created = await conversations.createConversation({});
        target = created.id;
      } catch (err) {
        // Nothing was sent: the question goes back where it was typed, and
        // the failure is said under the questions.
        setCreateError(describeApiError(err, { t, fallbackKey: 'app.failedCreateConversation' }));
        if (typed) setInputText(trimmed);
        return;
      }
      justCreatedRef.current = target;
      setConversationId(target);
    }

    try {
      trackMobile({ name: 'feature_engaged', props: { feature: 'chat_message_sent' } });
      const rotatedTo = await messagesHook.sendTurn(target, line);
      // `/reset` archives this thread and continues on a fresh one; the
      // activity's thread is the fresh one from then on.
      if (rotatedTo && rotatedTo !== target) setConversationId(rotatedTo);
    } finally {
      usageStatus.invalidate();
    }
  }, [conversationId, conversations, messagesHook, naming, setConversationId, t, usageStatus]);

  const sendTyped = useCallback((text: string) => send(text, true), [send]);
  const sendPrompt = useCallback((text: string) => void send(text, false), [send]);

  return (
    <KeyboardAvoidingView style={{ flex: 1 }} behavior="padding" keyboardVerticalOffset={headerHeight}>
      <ChatThread
        conversationId={conversationId}
        // The activity's thread is named by the screen's title: the activity.
        conversationTitle={naming.name}
        messagesHook={messagesHook}
        usageStatus={usageStatus}
        providerStatus={providerStatus}
        isLoading={false}
        inputText={inputText}
        onChangeInputText={setInputText}
        inputRef={inputRef}
        sendText={sendTyped}
        onScrollToBottom={followThread}
        header={
          <ActivityHeader
            detail={detail}
            sending={messagesHook.isSending}
            error={createError}
            onSend={sendPrompt}
          />
        }
      />
    </KeyboardAvoidingView>
  );
}

export function ActivityScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const params = useLocalSearchParams<{ provider?: string; activityId?: string }>();
  const provider = params.provider ?? '';
  const activityId = params.activityId ?? '';
  const detail = useActivityDetail(provider, activityId);

  if (detail.data === undefined) {
    let body: React.ReactNode;
    if (detail.notFound) {
      body = <EmptyState testID="activity-not-found">{t('home.activity.notFound')}</EmptyState>;
    } else if (detail.failed) {
      body = (
        <EmptyState
          action={{ label: t('common.retry'), onPress: detail.refetch, testID: 'activity-retry' }}
          testID="activity-failed"
        >
          {t('home.activity.loadFailed')}
        </EmptyState>
      );
    } else {
      body = (
        <View className="items-start px-4 py-3" testID="activity-loading">
          <ActivityIndicator color={colors.tokens.primary} />
        </View>
      );
    }
    return (
      <View className="flex-1 bg-background-primary py-4" testID="activity-screen">
        <Stack.Screen options={{ title: t('app.activity') }} />
        {body}
      </View>
    );
  }

  const data = detail.data;
  const sport = sportLabel(t, data.activity.sport_type);
  return (
    <View className="flex-1 bg-background-primary" testID="activity-screen">
      <Stack.Screen options={{ title: data.activity.name.trim() || sport }} />
      <ActivityThread provider={provider} activityId={activityId} detail={data} />
    </View>
  );
}
