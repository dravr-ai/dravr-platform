// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the RadioGroup primitive — the radiogroup and radio roles, each option's checked state, the key a press asks for
// ABOUTME: Pins the inks a choice is told apart by in both schemes: an outline-edged empty radio and a primary edge on the checked one, each at 3:1

import React from 'react';
import { StyleSheet, Text } from 'react-native';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { BOREAL_DARK, BOREAL_LIGHT, contrastHex as contrast } from '@pierre/shared-constants';

/** A rendered element, as the queries return it. */
type ReactTestInstance = ReturnType<typeof screen.getByTestId>;

jest.mock('nativewind', () => ({
  useColorScheme: () => ({ colorScheme: 'dark', setColorScheme: jest.fn() }),
}));

jest.mock('../../../services/api', () => ({
  userApi: { updateTheme: jest.fn().mockResolvedValue(undefined) },
}));

import { ThemeProvider, useTheme } from '../../../contexts/ThemeContext';
import { RadioGroup } from '../RadioGroup';

const APPEARANCE_KEY = 'pierre.appearance_pref';
const SCHEMES = ['light', 'dark'] as const;
type Scheme = (typeof SCHEMES)[number];
const TOKENS = { light: BOREAL_LIGHT, dark: BOREAL_DARK } as const;

const SPORTS = [
  { key: 'ride', label: 'Ride' },
  { key: 'run', label: 'Run' },
];
const FAMILIES = [
  { key: 'endurance_steady', label: 'Steady endurance' },
  { key: 'vo2max_short', label: 'Short VO2max intervals', description: 'VO2max, short intervals' },
];

function SchemeProbe() {
  return <Text testID="scheme-probe">{useTheme().scheme}</Text>;
}

/** The list's radio mark inside an option: the one view under it that draws an edge. */
function radioEdge(testID: string): string {
  const edged = screen
    .getByTestId(testID)
    .findAll(
      (node: ReactTestInstance) =>
        typeof node.type === 'string' && StyleSheet.flatten(node.props.style)?.borderColor !== undefined,
    );
  expect(edged).toHaveLength(1);
  return StyleSheet.flatten(edged[0].props.style).borderColor as string;
}

describe('RadioGroup', () => {
  it('is a radiogroup of radios, named by its label, reporting which one is checked', () => {
    render(<RadioGroup layout="segmented" label="Sport" options={SPORTS} value="run" onChange={jest.fn()} testID="sport" />);

    expect(screen.getByText('Sport')).toBeTruthy();
    // The group is not itself focusable — that would swallow its radios on iOS —
    // so it is found by its role prop rather than as an accessible element.
    const groups = screen
      .getByTestId('sport')
      .findAll((node: ReactTestInstance) => typeof node.type === 'string' && node.props.accessibilityRole === 'radiogroup');
    expect(groups).toHaveLength(1);
    const group = groups[0];
    expect(group.props.accessibilityLabel).toBe('Sport');
    const radios = within(group).getAllByRole('radio');
    expect(radios.map((radio) => radio.props.accessibilityLabel)).toEqual(['Ride', 'Run']);
    expect(screen.getByTestId('sport-ride').props.accessibilityState).toEqual({ checked: false });
    expect(screen.getByTestId('sport-run').props.accessibilityState).toEqual({ checked: true });
  });

  it('takes a name no label draws, read by a screen reader only', () => {
    render(
      <RadioGroup
        layout="segmented"
        accessibilityLabel="How to build the session"
        options={SPORTS}
        value="run"
        onChange={jest.fn()}
        testID="mode"
      />,
    );
    expect(screen.queryByText('How to build the session')).toBeNull();
    const group = screen
      .getByTestId('mode')
      .findAll((node: ReactTestInstance) => typeof node.type === 'string' && node.props.accessibilityRole === 'radiogroup');
    expect(group[0].props.accessibilityLabel).toBe('How to build the session');
  });

  it('asks for the key pressed, and reaches 44pt in both layouts', () => {
    const onChange = jest.fn();
    const { rerender } = render(
      <RadioGroup layout="segmented" options={SPORTS} value="run" onChange={onChange} testID="sport" />,
    );
    fireEvent.press(screen.getByTestId('sport-ride'));
    expect(onChange).toHaveBeenLastCalledWith('ride');
    expect(screen.getByTestId('sport-ride').props.className).toMatch(/\bmin-h-11\b/);

    rerender(<RadioGroup options={FAMILIES} value="endurance_steady" onChange={onChange} testID="family" />);
    fireEvent.press(screen.getByTestId('family-vo2max_short'));
    expect(onChange).toHaveBeenLastCalledWith('vo2max_short');
    expect(screen.getByTestId('family-vo2max_short').props.className).toMatch(/\bmin-h-11\b/);
  });

  it('marks the checked segment by its tint, its primary edge and its weight, not by colour alone', () => {
    render(<RadioGroup layout="segmented" options={SPORTS} value="run" onChange={jest.fn()} testID="sport" />);

    expect(screen.getByTestId('sport-run').props.className).toMatch(/\bborder-primary\b/);
    expect(screen.getByTestId('sport-ride').props.className).toMatch(/\bborder-transparent\b/);
    expect(screen.getByText('Run').props.className).toMatch(/\bfont-semibold\b/);
    expect(screen.getByText('Ride').props.className).toMatch(/\bfont-medium\b/);
  });

  it("shows an option's description in the list, and hands it to a screen reader as the hint", () => {
    render(<RadioGroup options={FAMILIES} value="vo2max_short" onChange={jest.fn()} testID="family" />);

    expect(screen.getByText('VO2max, short intervals')).toBeTruthy();
    expect(screen.getByTestId('family-vo2max_short').props.accessibilityHint).toBe('VO2max, short intervals');
    expect(screen.getByTestId('family-endurance_steady').props.accessibilityHint).toBeUndefined();
  });

  it.each(SCHEMES)('edges the empty radio in outline and the checked one in primary, each 3:1 on the sheet, %s', async (scheme) => {
    await AsyncStorage.setItem(APPEARANCE_KEY, scheme);
    render(
      <ThemeProvider>
        <SchemeProbe />
        <RadioGroup options={FAMILIES} value="vo2max_short" onChange={jest.fn()} testID="family" />
      </ThemeProvider>,
    );
    await waitFor(() => expect(screen.getByTestId('scheme-probe')).toHaveTextContent(scheme));

    const tokens = TOKENS[scheme];
    expect(radioEdge('family-endurance_steady')).toBe(tokens.outline);
    expect(radioEdge('family-vo2max_short')).toBe(tokens.primary);
    // The sheet's ground is surface-container-low; both edges are the control's only mark (WCAG 1.4.11).
    expect(contrast(tokens.outline, tokens.surfaceContainerLow)).toBeGreaterThanOrEqual(3);
    expect(contrast(tokens.primary, tokens.surfaceContainerLow)).toBeGreaterThanOrEqual(3);
  });
});
