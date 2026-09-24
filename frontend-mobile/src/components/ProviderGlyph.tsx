// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The face a provider row draws before its name — the brand glyph, or an initials circle when no mark exists for the id
// ABOUTME: Shared by every provider row and sheet so one provider wears one face, inked per scheme from the shared glyph-ink table

import React from 'react';
import { avatarSlot, initialsFor } from '@pierre/chat-utils';
import { providerGlyphInk, useTheme, useThemeColors } from '../constants/theme';
import { InitialsAvatar } from './ui';
import { providerGlyph } from './icons/BrandIcons';

/** The glyph slot's edge: every brand mark and the initials circle share it, so the names line up. */
export const GLYPH_SIZE = 24;

interface ProviderGlyphProps {
  /** The provider id the server reports, e.g. `sciotte`, `whoop`. */
  providerId: string;
  /** The provider's display name; the initials come from it when no mark exists. */
  label: string;
  size?: number;
}

/**
 * The brand mark for a provider id, or an initials circle when
 * `providerGlyph` knows no mark for it — a provider the server reports that
 * the glyph map has not caught up with still gets a face, in a slot colour
 * rather than a brand colour it does not have.
 *
 * The mark's ink is the shared `PROVIDER_GLYPH_INK` answer for the active
 * scheme: the brand colour where it clears the 3:1 icon floor on the canvas,
 * the body ink where it does not (TrainingPeaks' blue on the dark canvas,
 * WHOOP's green on the light one).
 */
export function ProviderGlyph({ providerId, label, size = GLYPH_SIZE }: ProviderGlyphProps) {
  const { scheme } = useTheme();
  const colors = useThemeColors();
  const Glyph = providerGlyph(providerId);
  if (Glyph) {
    return <Glyph size={size} color={providerGlyphInk(providerId, scheme) ?? colors.text.primary} />;
  }
  return (
    <InitialsAvatar
      initials={initialsFor(label)}
      slot={avatarSlot({ id: providerId, agent_id: null, group_id: null })}
      size={size}
    />
  );
}
