// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The day editor's head on the phone — the session's title as a headline field over its summary line, and the sport chips beside "Other…"
// ABOUTME: One sport row for both modes: the one-tap sports, the day's own word, and a sport the coach names in a field of its own (carnet#1727)

import React from 'react';
import { ScrollView, Text, View } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import { chosenSport, type HandSession, handSummary, SESSION_EDITOR_KEYS, sportLabel } from '@pierre/ui-logic';
import { Chip, HeadlineInput, Input } from '../../components/ui';

interface SessionSummaryProps {
  /** The session's sport word; left out of the line while it is empty. */
  sport: string;
  minutes: number | null;
  span?: string | null;
}

/** The line under the title: the sport as the card prints it, the minutes, the span its targets cover — each only when there is one. */
function SessionSummary({ sport, minutes, span }: SessionSummaryProps) {
  const { t } = useTranslation();
  const parts: React.ReactNode[] = [];
  if (sport.trim().length > 0) parts.push(sportLabel(t, sport.trim()));
  if (minutes !== null) {
    parts.push(
      <Text key="minutes" className="font-mono text-text-primary">
        {t(SESSION_EDITOR_KEYS.minutes, { count: minutes })}
      </Text>,
    );
  }
  if (span) parts.push(span);
  if (parts.length === 0) return null;
  return (
    <Text className="mt-2 text-sm text-text-secondary" testID="session-editor-summary">
      {parts.map((part, index) => (
        <React.Fragment key={index}>
          {index > 0 ? ' · ' : null}
          {part}
        </React.Fragment>
      ))}
    </Text>
  );
}

interface SessionTitleHeaderProps {
  session: HandSession;
  onTitle: (workout: string) => void;
}

/**
 * The session written by hand: its title as a large inline field on a
 * dashed underline, named "Title" for a screen reader, over the summary
 * line the steps total.
 */
export function SessionTitleHeader({ session, onTitle }: SessionTitleHeaderProps) {
  const { t } = useTranslation();
  const { minutes, span } = handSummary(session);
  return (
    <View className="mb-4 px-4" testID="session-editor-header">
      <HeadlineInput
        value={session.workout}
        onChangeText={onTitle}
        placeholder={t(SESSION_EDITOR_KEYS.titlePlaceholder)}
        accessibilityLabel={t(SESSION_EDITOR_KEYS.workout)}
        returnKeyType="done"
        testID="session-editor-workout"
      />
      <SessionSummary sport={session.sport} minutes={minutes} span={span} />
    </View>
  );
}

interface FamilyTitleHeaderProps {
  /** The format the build resolves to, by its name; `undefined` while the catalogue is read. */
  presetName: string | undefined;
  sport: string;
  /** The minutes the kernel's preview measured; `undefined` until it has. */
  previewMinutes: number | undefined;
}

/** A session built from a family: the format's name, which the coach does not edit, over the sport and the preview's minutes. */
export function FamilyTitleHeader({ presetName, sport, previewMinutes }: FamilyTitleHeaderProps) {
  return (
    <View className="mb-4 px-4" testID="session-editor-header">
      {presetName ? (
        <Text className="font-display text-2xl text-text-primary" accessibilityRole="header" testID="session-editor-preset-name">
          {presetName}
        </Text>
      ) : null}
      <SessionSummary sport={sport} minutes={previewMinutes ?? null} />
    </View>
  );
}

interface SportChipsProps {
  /** The chips, fixed for the sheet's life (`sportChoices`), so a pick never reorders the row under the thumb. */
  choices: readonly string[];
  /** The session's sport word. */
  sport: string;
  /** Whether the coach is naming a sport of their own: "Other…" is picked and its field shows. */
  otherOpen: boolean;
  onPick: (sport: string) => void;
  onOther: () => void;
  /** The sport the coach types in "Other…"'s field: the session's sport word itself. */
  onOtherName: (sport: string) => void;
}

/**
 * One horizontal row of sport chips, then "Other…" with its dashed edge.
 * The picked chip is the session's own sport, or "Other…" while the coach
 * names one, whose field then holds the sport word as typed.
 */
export function SportChips({ choices, sport, otherOpen, onPick, onOther, onOtherName }: SportChipsProps) {
  const { t } = useTranslation();
  const picked = otherOpen ? null : chosenSport(sport, choices);
  return (
    <View className="mb-4" testID="session-editor-sport">
      <ScrollView
        horizontal
        showsHorizontalScrollIndicator={false}
        keyboardShouldPersistTaps="handled"
        accessibilityLabel={t(SESSION_EDITOR_KEYS.sport)}
        contentContainerClassName="gap-2 px-4"
      >
        {choices.map((choice) => (
          <Chip
            key={choice}
            label={sportLabel(t, choice)}
            selected={picked === choice}
            onPress={() => onPick(choice)}
            testID={`session-editor-sport-${choice}`}
          />
        ))}
        <Chip
          label={t(SESSION_EDITOR_KEYS.sportOther)}
          selected={otherOpen}
          onPress={onOther}
          dashed
          testID="session-editor-sport-other"
        />
      </ScrollView>
      {otherOpen ? (
        <View className="mt-3 px-4">
          <Input
            label={t(SESSION_EDITOR_KEYS.sportOtherName)}
            value={sport}
            onChangeText={onOtherName}
            accessibilityLabel={t(SESSION_EDITOR_KEYS.sportOtherName)}
            autoFocus
            returnKeyType="done"
            testID="session-editor-sport-other-name"
          />
        </View>
      ) : null}
    </View>
  );
}
