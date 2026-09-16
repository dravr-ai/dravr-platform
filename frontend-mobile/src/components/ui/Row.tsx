// ABOUTME: The settings row — 52 tall (44 compact), a 16 title on the left, a 13 hint inline on the right, an 18 chevron when it navigates
// ABOUTME: The faint hairline sits on the inner column so it insets to the text; the last row of a group carries none (DESIGN.md §10)

import React, { type ReactNode } from 'react';
import {
  Pressable,
  StyleSheet,
  Text,
  View,
  type AccessibilityRole,
  type AccessibilityState,
} from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useThemeColors } from '../../constants/theme';

export interface RowProps {
  /** The row's name, reading text at 16. */
  title: string;
  /** A second line under the title, interface text on the secondary ink. */
  subtitle?: string;
  /** The current state, inline on the right in the tertiary ink; truncates before the title does. */
  hint?: string;
  /** Lands on the hint `Text`, for a spec that reads the state by id. */
  hintTestID?: string;
  /** A number that gets compared — a count, a time — in `font-mono tabular-nums`. */
  value?: string;
  /** A control on the trailing side: a switch, a badge. Suppresses the default chevron. */
  trailing?: ReactNode;
  onPress?: () => void;
  onLongPress?: () => void;
  /** Defaults to true when the row presses and carries neither `trailing` nor `value`. */
  showChevron?: boolean;
  /** 44 tall instead of 52, for a fact row that is read rather than tapped. */
  compact?: boolean;
  /** The last row of its group draws no hairline under itself. */
  last?: boolean;
  testID?: string;
  /** `button` by default when the row presses. */
  accessibilityRole?: AccessibilityRole;
  accessibilityState?: AccessibilityState;
  accessibilityLabel?: string;
}

/**
 * The outer element pays the screen's 16 inset; the inner one is the row
 * proper, so its hairline runs from the text's left edge to the pane's right
 * edge, the way the chat row's does. The title block is `flex-1` and the hint
 * `shrink`, so a long hint gives way first and the title never does. There is
 * no pressed style: the athlete side of Boreal gives no feedback flash on a
 * row, the screen it opens is the feedback.
 */
export function Row({
  title,
  subtitle,
  hint,
  hintTestID,
  value,
  trailing,
  onPress,
  onLongPress,
  showChevron,
  compact = false,
  last = false,
  testID,
  accessibilityRole,
  accessibilityState,
  accessibilityLabel,
}: RowProps) {
  const colors = useThemeColors();
  const pressable = onPress !== undefined || onLongPress !== undefined;
  const chevron = showChevron ?? (onPress !== undefined && trailing === undefined && value === undefined);

  const inner = (
    <View
      className={[
        'flex-row items-center',
        compact ? 'min-h-11' : 'min-h-[52px]',
        last ? '' : 'border-b border-border-faint',
      ]
        .filter(Boolean)
        .join(' ')}
      style={last ? undefined : { borderBottomWidth: StyleSheet.hairlineWidth }}
      testID={testID ? `${testID}-inner` : undefined}
    >
      {/* With a hint the title keeps its words and the hint gives way: the
          hint is the line that truncates, never the pane's name. Without one
          the title block takes the row and wraps a long subtitle. */}
      <View className={hint !== undefined ? 'shrink-0 max-w-[70%] py-2' : 'flex-1 min-w-0 py-2'}>
        <Text className="text-base text-text-primary">{title}</Text>
        {subtitle !== undefined && <Text className="text-sm text-text-secondary">{subtitle}</Text>}
      </View>
      {(hint !== undefined || value !== undefined || trailing !== undefined || chevron) && (
        <View
          className={
            hint !== undefined
              ? 'flex-1 min-w-0 flex-row items-center justify-end gap-1.5 ml-3'
              : 'flex-row items-center gap-1.5 ml-3 shrink'
          }
        >
          {hint !== undefined && (
            <Text
              className="shrink text-right text-sm text-text-tertiary"
              numberOfLines={1}
              ellipsizeMode="tail"
              testID={hintTestID}
            >
              {hint}
            </Text>
          )}
          {value !== undefined && (
            <Text className="text-sm font-mono tabular-nums text-text-secondary">{value}</Text>
          )}
          {trailing}
          {chevron && <Feather name="chevron-right" size={18} color={colors.text.tertiary} />}
        </View>
      )}
    </View>
  );

  if (!pressable) {
    return (
      <View
        className="px-4"
        testID={testID}
        accessibilityRole={accessibilityRole}
        accessibilityState={accessibilityState}
        accessibilityLabel={accessibilityLabel}
      >
        {inner}
      </View>
    );
  }

  return (
    <Pressable
      className="px-4"
      onPress={onPress}
      onLongPress={onLongPress}
      testID={testID}
      accessibilityRole={accessibilityRole ?? 'button'}
      accessibilityState={accessibilityState}
      accessibilityLabel={accessibilityLabel}
    >
      {inner}
    </Pressable>
  );
}
