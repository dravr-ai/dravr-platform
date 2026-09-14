// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the provider brand glyphs — every provider id resolves to an SVG mark, an unknown id to nothing
// ABOUTME: Pins each glyph's default colour to PROVIDER_COLORS, the one source the Strava hex used to duplicate

import React from 'react';
import { render } from '@testing-library/react-native';
import Svg, { Circle, Rect } from 'react-native-svg';

import {
  GarminLogo,
  IntervalsIcuLogo,
  StravaLogo,
  WhoopLogo,
  providerGlyph,
} from '../src/components/icons/BrandIcons';
import { PROVIDER_COLORS } from '../src/constants/theme';

describe('providerGlyph', () => {
  // Turns red if a provider the connection screens list loses its mark, or
  // if the captured sciotte ids stop sharing the brand's glyph.
  it.each([
    ['sciotte', StravaLogo],
    ['strava', StravaLogo],
    ['sciotte_garmin', GarminLogo],
    ['garmin', GarminLogo],
    ['whoop', WhoopLogo],
    ['intervals_icu', IntervalsIcuLogo],
  ])('%s resolves to a component that renders an Svg', (providerId, expected) => {
    const Glyph = providerGlyph(providerId);
    expect(Glyph).toBe(expected);
    if (Glyph === null) {
      throw new Error(`${providerId} resolved to no glyph`);
    }

    const { UNSAFE_getByType } = render(<Glyph size={20} />);
    const svg = UNSAFE_getByType(Svg);
    expect(svg.props.width).toBe(20);
    expect(svg.props.height).toBe(20);
  });

  it('an unknown id resolves to null', () => {
    expect(providerGlyph('fitbit')).toBeNull();
    expect(providerGlyph('')).toBeNull();
    // Object prototype names are not providers either.
    expect(providerGlyph('constructor')).toBeNull();
  });
});

describe('brand glyph default colours', () => {
  it('Strava fills with the shared Strava colour', () => {
    const { UNSAFE_getByType } = render(<StravaLogo />);
    expect(UNSAFE_getByType(Svg).props.fill).toBe(PROVIDER_COLORS.strava);
  });

  it('Garmin strokes with the shared Garmin colour', () => {
    const { UNSAFE_getByType } = render(<GarminLogo />);
    expect(UNSAFE_getByType(Circle).props.stroke).toBe(PROVIDER_COLORS.garmin);
  });

  it('Whoop strokes its band and fills its sensor with the shared Whoop colour', () => {
    const { UNSAFE_getByType } = render(<WhoopLogo />);
    expect(UNSAFE_getByType(Rect).props.stroke).toBe(PROVIDER_COLORS.whoop);
    expect(UNSAFE_getByType(Circle).props.fill).toBe(PROVIDER_COLORS.whoop);
  });

  it('intervals.icu fills its bars with the shared intervals.icu colour', () => {
    const { UNSAFE_getByType, UNSAFE_getAllByType } = render(<IntervalsIcuLogo />);
    expect(UNSAFE_getByType(Svg).props.fill).toBe(PROVIDER_COLORS.intervals_icu);
    expect(UNSAFE_getAllByType(Rect)).toHaveLength(3);
  });

  // The login modal draws every glyph in white on its brand button; an
  // explicit colour must still win over the default.
  it('an explicit colour overrides the default', () => {
    const { UNSAFE_getByType } = render(<StravaLogo color={PROVIDER_COLORS.garmin} />);
    expect(UNSAFE_getByType(Svg).props.fill).toBe(PROVIDER_COLORS.garmin);
  });
});
