// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One of the agent's group replies as the human coach reviews it — who it went to, its words, what the checker found, a flag and a correction
// ABOUTME: Same server calls and states as the web Replies card: a withheld reply has no words and no actions, a failed write keeps the form open (carnet#1304)

import React, { useState } from 'react';
import { Alert, Pressable, Text, View } from 'react-native';
import Toast from 'react-native-toast-message';
import type { CoachReply, ReplyFlagReason, VerdictTone } from '@pierre/shared-types';
import { summarizeVerdicts } from '@pierre/shared-types';
import {
  MAX_CORRECTION_CHARS,
  MAX_FLAG_NOTE_CHARS,
  REPLY_FLAG_REASONS,
  REPLY_FLAG_REASON_LABEL_KEY,
  VERDICT_SPOKEN_SEPARATOR,
  VERDICT_STATUS_LABEL_KEY,
  verdictChipLabel,
} from '@pierre/shared-constants';
import { formatDateTime } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';
import { Button, Input, StatusDot, type StatusDotTone } from '../../components/ui';
import { useReplyWrites } from '../../hooks/useCoachReplies';

/** Which form a reply has open. */
type OpenForm = 'flag' | 'correct' | null;

/** The dot each verdict rollup tone draws; a rhetorical or unverifiable claim is no alarm, so it stays quiet. */
const VERDICT_DOT_TONE: Record<VerdictTone, StatusDotTone> = {
  success: 'success',
  warning: 'warning',
  error: 'error',
  info: 'neutral',
  secondary: 'neutral',
};

/** The three flag reasons, one pressable each; the chosen one is filled. */
function ReasonPicker({ value, onChange }: { value: ReplyFlagReason; onChange: (reason: ReplyFlagReason) => void }) {
  const { t } = useTranslation();
  return (
    <View className="gap-2" accessibilityRole="radiogroup" testID="reply-flag-reasons">
      <Text className="text-sm font-medium text-text-secondary">{t('coachReview.reasonLabel')}</Text>
      <View className="flex-row flex-wrap gap-2">
        {REPLY_FLAG_REASONS.map((reason) => {
          const selected = reason === value;
          return (
            <Pressable
              key={reason}
              accessibilityRole="radio"
              accessibilityState={{ selected }}
              onPress={() => onChange(reason)}
              testID={`reply-flag-reason-${reason}`}
              className={`min-h-11 justify-center rounded-lg border px-4 ${
                selected ? 'border-primary bg-primary-container' : 'border-border-strong bg-transparent'
              }`}
            >
              <Text className={`text-sm font-medium ${selected ? 'text-on-primary-container' : 'text-text-primary'}`}>
                {t(REPLY_FLAG_REASON_LABEL_KEY[reason])}
              </Text>
            </Pressable>
          );
        })}
      </View>
    </View>
  );
}

/** One reply: who it went to, what it said, what the checker found, and the human coach's own steps. */
export function ReplyCard({ reply }: { reply: CoachReply }) {
  const { t, language } = useTranslation();
  const writes = useReplyWrites(reply);
  const [open, setOpen] = useState<OpenForm>(null);
  const [reason, setReason] = useState<ReplyFlagReason>('wrong');
  const [note, setNote] = useState('');
  const [correction, setCorrection] = useState('');
  const athlete = reply.athlete_name ?? t('coachReview.unnamedAthlete');
  const summary = summarizeVerdicts(reply.verdicts);

  const fail = (error: unknown, fallbackKey: string) =>
    Alert.alert(t('common.error'), describeApiError(error, { t, fallbackKey }));

  const submitFlag = () =>
    writes.flag.mutateAsync({ reason, note }).then(
      () => {
        setOpen(null);
        setNote('');
        Toast.show({ type: 'info', text1: t('coachReview.flagSent') });
      },
      (error: unknown) => fail(error, 'coachReview.flagFailed'),
    );

  const submitCorrection = () =>
    writes.correct.mutateAsync(correction).then(
      () => {
        setOpen(null);
        setCorrection('');
        Toast.show({ type: 'info', text1: t('coachReview.correctionSent') });
      },
      (error: unknown) => fail(error, 'coachReview.correctionFailed'),
    );

  return (
    <View className="gap-2 border-b border-border-faint px-4 py-4" testID={`coach-reply-${reply.id}`}>
      <View className="flex-row flex-wrap items-baseline justify-between gap-x-3">
        <Text className="min-w-0 shrink text-sm font-medium text-text-primary" numberOfLines={2}>
          {t('coachReview.replyTo', { name: athlete, group: reply.group_name })}
        </Text>
        <Text className="font-mono text-xs text-text-secondary">{formatDateTime(reply.created_at, language)}</Text>
      </View>

      {reply.withheld || reply.content === null ? (
        <Text className="text-sm italic text-text-secondary" testID="coach-reply-withheld">
          {t('coachReview.withheld')}
        </Text>
      ) : (
        <Text className="text-base text-text-primary" testID="coach-reply-content">
          {reply.content}
        </Text>
      )}

      {reply.unmatched_figures > 0 && (
        <View className="flex-row items-start gap-2" testID="coach-reply-unmatched">
          <View className="pt-1.5">
            <StatusDot tone="warning" />
          </View>
          <Text className="min-w-0 flex-1 text-xs text-text-primary">
            {t('coachReview.unmatchedFigures', { count: reply.unmatched_figures, name: athlete })}
          </Text>
        </View>
      )}

      {summary !== null && (
        <View className="gap-1.5" testID="coach-reply-verdicts">
          {/* The human coach's surfaces state a verdict as a dot beside the
              words, never as red or green text (DESIGN.md, Coach Home); the
              chat's tinted chip is a button that opens the sheet, and this line
              opens nothing. */}
          <View
            accessible
            accessibilityLabel={t('app.claimVerdictsStatus', {
              status: verdictChipLabel(t, summary, language, VERDICT_SPOKEN_SEPARATOR),
            })}
            className="mt-1 flex-row items-start gap-2"
            testID="coach-reply-verdict-chip"
          >
            <View className="pt-1">
              <StatusDot tone={VERDICT_DOT_TONE[summary.tone]} />
            </View>
            <Text className="min-w-0 flex-1 text-xs font-medium text-text-primary">{verdictChipLabel(t, summary, language)}</Text>
          </View>
          {reply.verdicts.map((verdict) => (
            <Text key={verdict.id} className="text-xs text-text-secondary" testID="coach-reply-verdict">
              {t(VERDICT_STATUS_LABEL_KEY[verdict.status])} · {verdict.claim_text}
            </Text>
          ))}
        </View>
      )}

      {reply.flags.map((f) => (
        <Text key={f.id} className="text-xs text-text-secondary" testID="coach-reply-flag">
          {t('coachReview.flagged', { reason: t(REPLY_FLAG_REASON_LABEL_KEY[f.reason]) })}
          {f.note ? ` — ${f.note}` : ''}
        </Text>
      ))}
      {reply.corrections.map((c) => (
        <View key={c.id} className="gap-0.5 border-l-2 border-primary pl-3" testID="coach-reply-correction">
          <Text className="text-xs font-medium text-primary">{t('coachReview.correctionPosted')}</Text>
          <Text className="text-sm text-text-primary">{c.content}</Text>
        </View>
      ))}

      {!reply.withheld && open === null && (
        <View className="mt-1 flex-row gap-2">
          <Button title={t('coachReview.flag')} variant="secondary" onPress={() => setOpen('flag')} testID="coach-reply-flag-open" />
          <Button title={t('coachReview.correct')} variant="secondary" onPress={() => setOpen('correct')} testID="coach-reply-correct-open" />
        </View>
      )}

      {open === 'flag' && (
        <View className="mt-2 gap-3" testID="coach-reply-flag-form">
          <ReasonPicker value={reason} onChange={setReason} />
          <Input
            label={t('coachReview.noteLabel')}
            value={note}
            onChangeText={setNote}
            maxLength={MAX_FLAG_NOTE_CHARS}
            multiline
            testID="coach-reply-flag-note"
          />
          <View className="flex-row justify-end gap-2">
            <Button title={t('common.cancel')} variant="ghost" onPress={() => setOpen(null)} testID="coach-reply-flag-cancel" />
            <Button
              title={t('coachReview.submitFlag')}
              loading={writes.flag.isPending}
              onPress={() => void submitFlag()}
              testID="coach-reply-flag-submit"
            />
          </View>
        </View>
      )}

      {open === 'correct' && (
        <View className="mt-2 gap-3" testID="coach-reply-correct-form">
          <Input
            label={t('coachReview.correctionLabel')}
            placeholder={t('coachReview.correctionPlaceholder', { name: athlete })}
            value={correction}
            onChangeText={setCorrection}
            maxLength={MAX_CORRECTION_CHARS}
            multiline
            testID="coach-reply-correction-input"
          />
          <View className="flex-row justify-end gap-2">
            <Button title={t('common.cancel')} variant="ghost" onPress={() => setOpen(null)} testID="coach-reply-correct-cancel" />
            <Button
              title={t('coachReview.submitCorrection')}
              loading={writes.correct.isPending}
              disabled={correction.trim().length === 0}
              onPress={() => void submitCorrection()}
              testID="coach-reply-correct-submit"
            />
          </View>
        </View>
      )}
    </View>
  );
}
