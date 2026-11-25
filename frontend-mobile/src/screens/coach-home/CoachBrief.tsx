// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's morning brief on the phone — a quiet status line atop À traiter, its settings sheet, and the athlete detail's leave-out action (carnet#1791)
// ABOUTME: Nothing draws until the brief is read, and nothing at all for a caller who coaches no one; every change saves at once and says so when refused

import React, { useMemo, useState } from 'react';
import { Alert, Pressable, ScrollView, Switch, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import type { CoachBrief, CoachBriefUpdate } from '@pierre/shared-types';
import { COACH_BRIEF_REASON_LABEL_KEY } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import {
  BRIEF_HOURS,
  BRIEF_REASON_KINDS,
  briefHourLabel,
  briefStatusLine,
  briefStatusWords,
  isBriefMuted,
  mutedAthletes,
  withMuted,
  withReason,
} from '@pierre/ui-logic';
import { Button, RadioGroup, Row, Section, Sheet, switchColors } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useCoachBrief, useUpdateCoachBrief } from '../../hooks/useCoachBrief';
import { useCoachRoster } from '../../hooks/useCoachRoster';

/** The hour picker's choices, 05:00 to 23:00. */
const HOUR_OPTIONS = BRIEF_HOURS.map((hour) => ({ key: String(hour), label: briefHourLabel(hour) }));

/**
 * The brief's line at the head of À traiter, beside "See all": its name, then
 * where it stands and when it runs, at the caption size, so it reads as a
 * note on the queue and never as a row of it. A tap opens the settings.
 */
export function CoachBriefControl() {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const { brief } = useCoachBrief(true);
  const [open, setOpen] = useState(false);
  if (brief === undefined) return null;
  const title = t('coachHome.brief.title');
  const line = briefStatusLine(briefStatusWords(t, language, brief));
  return (
    <>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${title}, ${line}`}
        accessibilityHint={t('coachHome.brief.settings')}
        onPress={() => setOpen(true)}
        testID="coach-brief"
        className="min-h-11 min-w-0 flex-1 flex-row items-center gap-2 py-1.5 pr-3"
      >
        <Feather name="sunrise" size={16} color={colors.text.secondary} />
        <View className="min-w-0 flex-1">
          <Text className="text-xs font-medium text-text-primary">{title}</Text>
          <Text className="text-xs text-text-secondary" testID="coach-brief-status">
            {line}
          </Text>
        </View>
      </Pressable>
      <Sheet visible={open} onClose={() => setOpen(false)} flush testID="coach-brief-sheet" backdropTestID="coach-brief-backdrop">
        {open && <CoachBriefSettings brief={brief} />}
      </Sheet>
    </>
  );
}

/**
 * The brief's settings: pause or resume, its hour, the reasons it checks and
 * the athletes it leaves out. Each change saves on its own, shown at once; a
 * refused one is rolled back and the sheet says so until the next change.
 */
function CoachBriefSettings({ brief }: { brief: CoachBrief }) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const { roster } = useCoachRoster(true);
  const { update } = useUpdateCoachBrief();
  const [failed, setFailed] = useState(false);
  const [hoursOpen, setHoursOpen] = useState(false);
  const paused = brief.state === 'paused';
  const leftOut = useMemo(() => mutedAthletes(brief.muted, roster?.athletes ?? []), [brief.muted, roster]);

  const save = (change: CoachBriefUpdate) => {
    setFailed(false);
    update(change).catch(() => setFailed(true));
  };

  return (
    <ScrollView contentContainerClassName="gap-8 pb-4" testID="coach-brief-scroll">
      <View className="gap-3 px-4">
        <Text accessibilityRole="header" className="text-lg font-semibold text-text-primary">
          {t('coachHome.brief.settings')}
        </Text>
        <View className="flex-row items-center gap-3">
          <Text className="min-w-0 flex-1 text-sm text-text-secondary" testID="coach-brief-sheet-status">
            {briefStatusLine(briefStatusWords(t, language, brief))}
          </Text>
          <Button
            title={paused ? t('coachHome.brief.resume') : t('coachHome.brief.pause')}
            variant={paused ? 'primary' : 'secondary'}
            onPress={() => save({ paused: !paused })}
            testID="coach-brief-pause"
          />
        </View>
        {failed && (
          <Text accessibilityRole="alert" className="text-sm text-error" testID="coach-brief-save-failed">
            {t('coachHome.brief.save_failed')}
          </Text>
        )}
      </View>

      {/* The hour is a radio list of nineteen, folded behind its row until asked for. */}
      <View>
        <Row
          title={t('coachHome.brief.hour_label')}
          value={briefHourLabel(brief.hour)}
          showChevron
          onPress={() => setHoursOpen((shown) => !shown)}
          accessibilityState={{ expanded: hoursOpen }}
          last={!hoursOpen}
          testID="coach-brief-hour"
        />
        {hoursOpen && (
          <View className="px-4 pt-2">
            <RadioGroup
              layout="list"
              accessibilityLabel={t('coachHome.brief.hour_label')}
              options={HOUR_OPTIONS}
              value={String(brief.hour)}
              onChange={(key) => {
                setHoursOpen(false);
                save({ hour: Number(key) });
              }}
              testID="coach-brief-hours"
            />
          </View>
        )}
      </View>

      <Section title={t('coachHome.brief.reasons_label')} testID="coach-brief-reasons">
        {BRIEF_REASON_KINDS.map((kind, index) => {
          const label = t(COACH_BRIEF_REASON_LABEL_KEY[kind]);
          const on = !brief.reasons_off.includes(kind);
          return (
            <Row
              key={kind}
              title={label}
              last={index === BRIEF_REASON_KINDS.length - 1}
              trailing={
                <Switch
                  value={on}
                  onValueChange={(next) => save({ reasons_off: withReason(brief.reasons_off, kind, next) })}
                  accessibilityLabel={label}
                  {...switchColors(colors.tokens, on)}
                  testID={`coach-brief-reason-${kind}`}
                />
              }
            />
          );
        })}
      </Section>

      <Section title={t('coachHome.brief.muted_label')} testID="coach-brief-muted">
        {leftOut.length === 0 ? (
          <Text className="px-4 text-sm text-text-secondary" testID="coach-brief-muted-none">
            {t('coachHome.brief.muted_none')}
          </Text>
        ) : (
          leftOut.map((athlete, index) => (
            <Row
              key={athlete.user_id}
              title={athlete.name}
              last={index === leftOut.length - 1}
              testID={`coach-brief-muted-${athlete.user_id}`}
              trailing={
                <Pressable
                  accessibilityRole="button"
                  // Every row has one: the name says which athlete it puts back.
                  accessibilityLabel={`${t('coachHome.brief.unmute')}, ${athlete.name}`}
                  onPress={() => save({ muted: withMuted(brief.muted, athlete.user_id, false) })}
                  testID={`coach-brief-unmute-${athlete.user_id}`}
                  className="min-h-11 justify-center pl-3"
                >
                  <Text className="text-sm font-medium text-primary">{t('coachHome.brief.unmute')}</Text>
                </Pressable>
              }
            />
          ))
        )}
      </Section>
    </ScrollView>
  );
}

/**
 * The athlete detail's way to leave this athlete out of the brief, or put
 * them back, at the foot of the page: an occasional choice, kept apart from
 * the actions that handle today's reason. It sends the whole `muted` list, as
 * the PUT replaces it, and an alert says so when the server refuses.
 */
export function CoachBriefMuteAction({ athleteId }: { athleteId: string }) {
  const { t } = useTranslation();
  const { brief } = useCoachBrief(true);
  const { update } = useUpdateCoachBrief();
  if (brief === undefined) return null;
  const muted = isBriefMuted(brief, athleteId);
  return (
    <View className="px-4">
      <Button
        title={muted ? t('coachHome.brief.unmute') : t('coachHome.brief.mute')}
        variant="ghost"
        fullWidth
        testID={muted ? 'coach-brief-unmute' : 'coach-brief-mute'}
        onPress={() => {
          update({ muted: withMuted(brief.muted, athleteId, !muted) }).catch(() =>
            Alert.alert(t('common.error'), t('coachHome.brief.save_failed')),
          );
        }}
      />
    </View>
  );
}
