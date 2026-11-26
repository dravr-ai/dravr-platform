// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete workspace's two weeks on the phone — each week a section, each date a group of hairline rows: its sessions, then « + Séance »
// ABOUTME: A date the draft changes sits on a faint primary wash with « Proposé », Accepter and Refuser; a session kept as the coach's call offers its withdrawal (carnet#1818)

import React from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import type { PlanDay } from '@pierre/shared-types';
import { COACH_CALL_KEYS } from '@pierre/chat-utils';
import {
  MAX_SESSIONS_PER_DATE,
  PLAN_WORKSPACE_KEYS,
  ROW_DATE,
  SESSION_EDITOR_KEYS,
  canAddSession,
  formatCivilDate,
  newSessionDay,
  type PlanCalendarDay,
  type PlanCalendarWeek,
} from '@pierre/ui-logic';
import { Badge } from '../../../components/ui';
import { useThemeColors } from '../../../constants/theme';
import { DayRow, WeekHeading, type SessionEditorTarget } from '../../chat/WorkoutPlanCard';

/** The long date a screen reader hears for a date's heading: « lundi 23 novembre ». */
const LONG_DATE: Intl.DateTimeFormatOptions = { weekday: 'long', day: 'numeric', month: 'long' };

/**
 * The wash a proposed date sits on: it marks where the draft changes the
 * plan, under content that keeps its own ink (DESIGN.md "Alerts", "A wash is
 * not an alert") — faint, no edge. The phone has no wash table of its own,
 * so the class is the /5 step of the one accent.
 */
const PROPOSED_WASH = 'bg-primary/5';

interface WorkspaceCalendarProps {
  weeks: PlanCalendarWeek[];
  /** Whether the coach accepted the proposal on a date this visit (decision W1). */
  isAccepted: (day: PlanCalendarDay) => boolean;
  onAccept: (day: PlanCalendarDay) => void;
  /** « Refuser »: put the live plan back on the date. */
  onRefuse: (date: string) => void;
  /** The date whose refusal is being written, while one is. */
  refusing: string | null;
  /** « Retirer mon choix » on a session kept as the coach's call; `position` is its place among the date's entries. */
  onWithdrawCall: (day: PlanDay, position: number) => void;
  /** A call is being saved: every withdrawal waits. */
  callSaving: boolean;
  /** Open the session editor on a session, or on a blank one after a date's sessions. */
  onEdit: (target: SessionEditorTarget) => void;
}

/**
 * One row of a date's group: the pane's 16 inset outside, the faint hairline
 * on the inner column so it insets to the text, none under the group's last
 * row (DESIGN.md "Phone").
 */
function GroupRow({ last, children }: { last: boolean; children: React.ReactNode }) {
  return (
    <View className="px-4">
      <View
        className={last ? undefined : 'border-b border-border-faint'}
        style={last ? undefined : { borderBottomWidth: StyleSheet.hairlineWidth }}
      >
        {children}
      </View>
    </View>
  );
}

/**
 * One session of a date: a press opening its editor, drawn as the plan
 * card's row is — sport, length, marks, the coach's call — then, when the
 * coach kept it as their call, the line that withdraws it. Past or ahead, a
 * session keeps the row's own inks; only the date's heading goes quieter.
 */
function SessionRow({
  day,
  position,
  entries,
  onEdit,
  onWithdrawCall,
  callSaving,
}: {
  day: PlanDay;
  position: number;
  entries: PlanDay[];
  onEdit: (target: SessionEditorTarget) => void;
  onWithdrawCall: (day: PlanDay, position: number) => void;
  callSaving: boolean;
}) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const title = day.rest ? t('chat.restDay') : day.workout;
  const kept = day.coach_call === true && !day.rest;
  return (
    <GroupRow last={false}>
      <Pressable
        onPress={() => onEdit({ day, entries, mode: 'edit' })}
        accessibilityRole="button"
        accessibilityLabel={t(PLAN_WORKSPACE_KEYS.editSession, {
          session: title,
          date: formatCivilDate(day.date, language, ROW_DATE),
        })}
        // A rest day is one line: the press keeps the 44 pt floor all the same.
        className="min-h-11 flex-row items-start gap-2 pb-2"
        testID={`workspace-session-${day.date}-${position}`}
      >
        <View className="flex-1">
          <DayRow day={day} position={position} dateColumn={false} />
        </View>
        <View className="mt-2.5">
          <Feather name="edit-2" size={14} color={colors.tokens.primary} />
        </View>
      </Pressable>
      {kept ? (
        <Pressable
          onPress={() => {
            if (!callSaving) onWithdrawCall(day, position);
          }}
          accessibilityRole="button"
          accessibilityHint={title}
          accessibilityState={{ disabled: callSaving, busy: callSaving }}
          hitSlop={10}
          className={`min-h-11 justify-center self-start ${callSaving ? 'opacity-50' : ''}`}
          testID={`workspace-call-undo-${day.date}-${position}`}
        >
          <Text className="text-sm font-medium text-primary">{t(COACH_CALL_KEYS.undo)}</Text>
        </Pressable>
      ) : null}
    </GroupRow>
  );
}

/**
 * One date, read like a section: its heading, « Proposé » when the draft
 * changes it, its sessions as hairline rows, the proposal's two answers and
 * the « + Séance » row last. No box: a proposed date sits on the faint
 * primary wash, today's heading is in the accent, a past date's is quieter.
 */
function DateGroup({
  day,
  accepted,
  onAccept,
  onRefuse,
  refusing,
  onWithdrawCall,
  callSaving,
  onEdit,
}: {
  day: PlanCalendarDay;
  accepted: boolean;
  onAccept: () => void;
  onRefuse: () => void;
  refusing: boolean;
  onWithdrawCall: (day: PlanDay, position: number) => void;
  callSaving: boolean;
  onEdit: (target: SessionEditorTarget) => void;
}) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const today = day.timing === 'today';
  const past = day.timing === 'past';
  const full = !canAddSession(day.entries);
  const rowDate = formatCivilDate(day.date, language, ROW_DATE);
  // Where the proposal stands, said with the date's heading, so a screen
  // reader moving heading by heading hears which dates are proposed.
  const proposal = day.proposed ? t(accepted ? PLAN_WORKSPACE_KEYS.accepted : PLAN_WORKSPACE_KEYS.proposed) : null;
  // The editor joins a new session to the date: its last entry, or a blank
  // session on a date the plan holds nothing on.
  const addTarget: SessionEditorTarget = {
    day: day.entries[day.entries.length - 1] ?? newSessionDay(day.date, day.entries),
    entries: day.entries,
    mode: 'add',
  };
  return (
    <View className={`py-2 ${day.proposed ? PROPOSED_WASH : ''}`} testID={`workspace-date-${day.date}`}>
      <View className="min-h-8 flex-row flex-wrap items-center justify-between gap-2 px-4">
        <Text
          accessibilityRole="header"
          accessibilityLabel={`${formatCivilDate(day.date, language, LONG_DATE)}${proposal !== null ? `, ${proposal}` : ''}`}
          className={`font-mono text-sm tabular-nums ${today ? 'font-semibold text-primary' : past ? 'text-text-tertiary' : 'font-semibold text-text-primary'}`}
        >
          {rowDate}
        </Text>
        {/* The heading says the same to a screen reader. */}
        {day.proposed ? (
          accepted ? (
            <View
              accessibilityElementsHidden
              importantForAccessibility="no-hide-descendants"
              className="flex-row items-center gap-1"
              testID="workspace-date-accepted"
            >
              <Feather name="check" size={12} color={colors.tokens.onSurface} />
              <Badge variant="neutral" label={t(PLAN_WORKSPACE_KEYS.accepted)} />
            </View>
          ) : (
            <View accessibilityElementsHidden importantForAccessibility="no-hide-descendants" testID="workspace-date-proposed">
              <Badge variant="primary" label={t(PLAN_WORKSPACE_KEYS.proposed)} />
            </View>
          )
        ) : null}
      </View>
      {day.entries.map((entry, position) => (
        <SessionRow
          key={`${entry.date}-${position}`}
          day={entry}
          position={position}
          entries={day.entries}
          onEdit={onEdit}
          onWithdrawCall={onWithdrawCall}
          callSaving={callSaving}
        />
      ))}
      {day.proposed ? (
        <GroupRow last={false}>
          <View className="flex-row gap-x-5">
            {!accepted ? (
              <Pressable
                onPress={onAccept}
                accessibilityRole="button"
                accessibilityHint={rowDate}
                hitSlop={10}
                className="min-h-11 justify-center"
                testID={`workspace-accept-${day.date}`}
              >
                <Text className="text-sm font-semibold text-primary">{t(PLAN_WORKSPACE_KEYS.accept)}</Text>
              </Pressable>
            ) : null}
            {/* Held back while the date is put back: a second press would ask twice. */}
            <Pressable
              onPress={() => {
                if (!refusing) onRefuse();
              }}
              accessibilityRole="button"
              accessibilityHint={rowDate}
              accessibilityState={{ disabled: refusing, busy: refusing }}
              hitSlop={10}
              className={`min-h-11 justify-center ${refusing ? 'opacity-50' : ''}`}
              testID={`workspace-refuse-${day.date}`}
            >
              <Text className="text-sm font-semibold text-text-secondary">{t(PLAN_WORKSPACE_KEYS.refuse)}</Text>
            </Pressable>
          </View>
        </GroupRow>
      ) : null}
      <GroupRow last>
        <Pressable
          onPress={() => onEdit(addTarget)}
          disabled={full}
          accessibilityRole="button"
          accessibilityLabel={t(SESSION_EDITOR_KEYS.addTitle, { date: rowDate })}
          accessibilityHint={full ? t(SESSION_EDITOR_KEYS.dateFull, { date: rowDate, max: MAX_SESSIONS_PER_DATE }) : undefined}
          accessibilityState={{ disabled: full }}
          className={`min-h-11 flex-row items-center gap-1 self-start ${full ? 'opacity-50' : ''}`}
          testID={`workspace-add-${day.date}`}
        >
          <Feather name="plus" size={14} color={colors.tokens.primary} />
          <Text className="text-sm font-semibold text-primary">{t(PLAN_WORKSPACE_KEYS.addShort)}</Text>
        </Pressable>
      </GroupRow>
    </View>
  );
}

/**
 * The fortnight as the coach works it on the phone: each week a section under
 * its heading, its dates groups separated by space, never by a box. Today's
 * heading is in the accent, a past date's quieter, and a date the draft
 * changes sits on a faint wash and says « Proposé » until the coach accepts
 * or refuses it.
 */
export function WorkspaceCalendar({
  weeks,
  isAccepted,
  onAccept,
  onRefuse,
  refusing,
  onWithdrawCall,
  callSaving,
  onEdit,
}: WorkspaceCalendarProps) {
  const currentIndex = weeks.findIndex((week) => week.week.current);
  return (
    <View className="gap-8" testID="workspace-calendar">
      {weeks.map((week, index) => (
        <View key={week.week.week_start} className="gap-3" testID={`workspace-week-${week.week.week_start}`}>
          <View className="px-4">
            <WeekHeading week={week.week} index={index} currentIndex={currentIndex} />
          </View>
          <View className="gap-2">
            {week.days.map((day) => (
              <DateGroup
                key={day.date}
                day={day}
                accepted={isAccepted(day)}
                onAccept={() => onAccept(day)}
                onRefuse={() => onRefuse(day.date)}
                refusing={refusing === day.date}
                onWithdrawCall={onWithdrawCall}
                callSaving={callSaving}
                onEdit={onEdit}
              />
            ))}
          </View>
        </View>
      ))}
    </View>
  );
}
