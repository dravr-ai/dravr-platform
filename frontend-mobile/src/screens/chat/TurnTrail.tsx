// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A long turn's steps on the phone — the live list with its timer and safe-to-leave note, and the line it folds into
// ABOUTME: Paints what @pierre/chat-utils' turn trail returns; every rule (labels, thresholds, the fold) lives there

import React, { useEffect, useRef, useState } from 'react';
import { AccessibilityInfo, Platform, Pressable, Text, View } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import {
  doneSteps,
  foldTurnTrail,
  TURN_STEP_KEYS,
  TURN_TRAIL_KEYS,
  turnCurrentLine,
  turnIndicator,
  type TranslatableText,
  type TurnStep,
  type TurnTrail,
} from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { useTurnElapsedMs } from '@pierre/ui-logic';
import { useThemeColors } from '../../constants/theme';

/** The size of a checked step's mark and of the fold's chevron. */
const MARK_SIZE = 14;

function useText(): (text: TranslatableText) => string {
  const { t } = useTranslation();
  return text => t(text.key, text.params);
}

function DoneStep({ step }: { step: TurnStep }) {
  const { t } = useTranslation();
  const text = useText();
  const colors = useThemeColors();
  const label = text(step.label);
  return (
    <View
      className="flex-row items-center gap-2"
      testID="turn-step-done"
      accessible
      accessibilityLabel={`${label}, ${t(TURN_TRAIL_KEYS.stepDone)}`}
    >
      <Ionicons name="checkmark" size={MARK_SIZE} color={colors.tokens.primary} />
      <Text className="flex-shrink text-sm text-text-secondary">{label}</Text>
    </View>
  );
}

interface LiveTurnProps {
  /** The turn's steps; `null` for a turn restored after a reload, whose steps are unknown. */
  trail: TurnTrail | null;
  /**
   * When the turn started (epoch ms): the restored turn's question time.
   * Defaults to the trail's own start, else to when this mounted.
   */
  startedAt?: number | null;
  /** The mark that says the turn is working, drawn before the current step. */
  glyph: React.ReactNode;
}

/**
 * Tell a screen reader the step changed — once per change, never per tick.
 *
 * Android reads a live region by itself; iOS has none, so the new line is
 * announced. The first line is not: the indicator appearing already says the
 * turn started.
 */
function useAnnounceStep(line: string): void {
  const previous = useRef<string | null>(null);
  useEffect(() => {
    const before = previous.current;
    previous.current = line;
    if (before !== null && before !== line && Platform.OS === 'ios') {
      AccessibilityInfo.announceForAccessibility(line);
    }
  }, [line]);
}

/**
 * The running turn, where its reply will be: the steps it has finished, each
 * checked, then the one it is on beside the working mark and, after a few
 * seconds, how long it has run — and, on a long turn, that the athlete may
 * leave and find the reply here.
 */
export function LiveTurnIndicator({ trail, startedAt, glyph }: LiveTurnProps) {
  const text = useText();
  const indicator = turnIndicator(useTurnElapsedMs(startedAt ?? trail?.startedAt));
  const current = text(trail ? turnCurrentLine(trail) : { key: TURN_STEP_KEYS.thinking });
  const done = trail ? doneSteps(trail) : [];
  useAnnounceStep(current);
  return (
    <View className="mb-4 gap-1" testID="turn-indicator">
      {done.map(step => (
        <DoneStep key={step.id} step={step} />
      ))}
      <View className="flex-row items-center gap-2" testID="turn-current-step">
        {glyph}
        <Text className="flex-shrink text-sm text-text-primary" accessibilityLiveRegion="polite">
          {current}
        </Text>
        {/* Outside the live region: a screen reader hears the step change,
            not every second the clock moves. */}
        {indicator.elapsed ? (
          <Text className="text-sm tabular-nums text-text-secondary" testID="turn-elapsed">
            {`· ${text(indicator.elapsed)}`}
          </Text>
        ) : null}
      </View>
      {indicator.safeToLeave ? (
        <Text className="mt-1 text-xs leading-5 text-text-secondary" testID="turn-safe-to-leave">
          {text(indicator.safeToLeave)}
        </Text>
      ) : null}
    </View>
  );
}

/**
 * A finished turn, folded into one line above its reply — "Réfléchi 7 min ·
 * 6 étapes" — that opens into the steps it took. Nothing for a quick answer
 * that called no tool.
 */
export function FoldedTurnTrail({ trail }: { trail: TurnTrail }) {
  const { t } = useTranslation();
  const text = useText();
  const colors = useThemeColors();
  const [open, setOpen] = useState(false);
  const folded = foldTurnTrail(trail);
  if (folded === null) return null;
  const line = t(folded.line.key, { ...folded.line.params, duration: text(folded.duration) });
  if (folded.steps.length === 0) {
    return (
      <Text className="mb-1 text-xs text-text-secondary" testID="turn-trail-folded">
        {line}
      </Text>
    );
  }
  return (
    <View className="mb-1" testID="turn-trail-folded">
      <Pressable
        onPress={() => setOpen(value => !value)}
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
        accessibilityHint={t(open ? TURN_TRAIL_KEYS.hideSteps : TURN_TRAIL_KEYS.showSteps)}
        className="min-h-11 flex-row items-center gap-1 self-start"
        testID="turn-trail-toggle"
      >
        <Ionicons
          name={open ? 'chevron-down' : 'chevron-forward'}
          size={MARK_SIZE}
          color={colors.tokens.onSurfaceVariant}
        />
        <Text className="text-xs text-text-secondary">{line}</Text>
      </Pressable>
      {open ? (
        <View
          className="gap-1 border-l border-border-faint pl-3"
          testID="turn-trail-steps"
        >
          {folded.steps.map(step => (
            <DoneStep key={step.id} step={step} />
          ))}
        </View>
      ) : null}
    </View>
  );
}
