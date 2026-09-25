// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the provider brand glyphs — every provider id resolves to an SVG mark, an unknown id to nothing
// ABOUTME: Pins that each mark draws in the ink its caller hands it — the scheme, not the mark, decides a brand colour

import React from 'react';
import { render } from '@testing-library/react-native';
import Svg, { Circle, Rect } from 'react-native-svg';

import {
  CorosLogo,
  GarminLogo,
  IntervalsIcuLogo,
  StravaLogo,
  TrainingPeaksLogo,
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
    ['sciotte_trainingpeaks', TrainingPeaksLogo],
    ['sciotte_coros', CorosLogo],
    ['coros', CorosLogo],
    ['whoop', WhoopLogo],
    ['intervals_icu', IntervalsIcuLogo],
  ])('%s resolves to a component that renders an Svg', (providerId, expected) => {
    const Glyph = providerGlyph(providerId);
    expect(Glyph).toBe(expected);
    if (Glyph === null) {
      throw new Error(`${providerId} resolved to no glyph`);
    }

    const { UNSAFE_getByType } = render(<Glyph size={20} color={PROVIDER_COLORS.strava} />);
    const svg = UNSAFE_getByType(Svg);
    expect(svg.props.width).toBe(20);
    expect(svg.props.height).toBe(20);
  });

  it('an unknown id resolves to null', () => {
    expect(providerGlyph('polar')).toBeNull();
    expect(providerGlyph('')).toBeNull();
    // Object prototype names are not providers either.
    expect(providerGlyph('constructor')).toBeNull();
  });
});

describe('brand marks draw in the ink they are handed', () => {
  // A mark has no default ink: TrainingPeaks' blue fails the dark canvas and
  // WHOOP's green the light one, so a caller that forgot the ink would ship a
  // mark that vanishes in one scheme. Each case hands a colour that is NOT the
  // brand's own, so a mark that ignored its prop and kept a baked-in brand hex
  // would fail here.
  const INK = '#123456';

  it('Strava fills its chevrons with the ink', () => {
    const { UNSAFE_getByType } = render(<StravaLogo color={INK} />);
    expect(UNSAFE_getByType(Svg).props.fill).toBe(INK);
  });

  it('Garmin strokes its ring with the ink', () => {
    const { UNSAFE_getByType } = render(<GarminLogo color={INK} />);
    expect(UNSAFE_getByType(Circle).props.stroke).toBe(INK);
  });

  it('TrainingPeaks fills its peaks with the ink', () => {
    const { UNSAFE_getByType } = render(<TrainingPeaksLogo color={INK} />);
    expect(UNSAFE_getByType(Svg).props.fill).toBe(INK);
  });

  it('COROS fills its official mark with the ink', () => {
    const { UNSAFE_getByType } = render(<CorosLogo color={INK} />);
    expect(UNSAFE_getByType(Svg).props.fill).toBe(INK);
    expect(UNSAFE_getByType(Svg).props.viewBox).toBe('0 0 1024 1024');
  });

  it('Whoop strokes its band and fills its sensor with the ink', () => {
    const { UNSAFE_getByType } = render(<WhoopLogo color={INK} />);
    expect(UNSAFE_getByType(Rect).props.stroke).toBe(INK);
    expect(UNSAFE_getByType(Circle).props.fill).toBe(INK);
  });

  it('intervals.icu fills its three bars with the ink', () => {
    const { UNSAFE_getByType, UNSAFE_getAllByType } = render(<IntervalsIcuLogo color={INK} />);
    expect(UNSAFE_getByType(Svg).props.fill).toBe(INK);
    expect(UNSAFE_getAllByType(Rect)).toHaveLength(3);
  });
});
