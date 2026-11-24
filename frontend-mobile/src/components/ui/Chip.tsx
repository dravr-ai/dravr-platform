// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A chip a form picks with one tap — a word, or a figure as a tile — 44 tall, its pick read by fill, edge and weight
// ABOUTME: A toggle button among buttons (selected) or one radio of a radiogroup (checked); a dashed edge for the chip that opens a field

import React from 'react';
import { Pressable, Text } from 'react-native';

export interface ChipProps {
  /** The visible word or figure. */
  label: string;
  selected: boolean;
  onPress: () => void;
  /** The accessible name, when the visible label is a symbol (`%`); else the label itself. */
  accessibilityLabel?: string;
  /**
   * `word`: a word (a sport) at the phone's chip radius, 4
   * (DESIGN.md "Shapes"); `tile`: a figure (`Z4`, `RPE`) in the mono face, at least
   * 44 square, radius 8 like the segments beside it.
   */
  shape?: 'word' | 'tile';
  /**
   * `button`: one of a row of toggle buttons, read as selected; `radio`: one
   * option of a group carrying the radiogroup role, read as checked.
   */
  role?: 'button' | 'radio';
  /** A dashed edge: the chip that opens a field of its own ("Other…"). */
  dashed?: boolean;
  testID?: string;
}

/**
 * The picked chip takes `primary-container` under its bound ink with a
 * `primary` edge and the semibold weight, so the pick reads by fill, edge and
 * weight rather than by colour alone; the others sit on the lowest container
 * with the control-outline hairline (DESIGN.md "Badges and chips").
 */
export function Chip({
  label,
  selected,
  onPress,
  accessibilityLabel,
  shape = 'word',
  role = 'button',
  dashed = false,
  testID,
}: ChipProps) {
  const edge = selected ? 'border-primary bg-primary-container' : 'border-border-strong bg-surface-container-lowest';
  const form = shape === 'word' ? 'rounded px-3.5' : 'min-w-11 rounded-lg px-2.5';
  const ink = selected ? 'font-semibold text-on-primary-container' : 'font-medium text-text-primary';
  return (
    <Pressable
      className={`min-h-11 flex-row items-center justify-center border ${form} ${edge}${dashed ? ' border-dashed' : ''}`}
      onPress={onPress}
      accessibilityRole={role}
      accessibilityState={role === 'radio' ? { checked: selected } : { selected }}
      accessibilityLabel={accessibilityLabel ?? label}
      testID={testID}
    >
      <Text className={`text-sm ${shape === 'tile' ? 'font-mono ' : ''}${ink}`} numberOfLines={1}>
        {label}
      </Text>
    </Pressable>
  );
}
