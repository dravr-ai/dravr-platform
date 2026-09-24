// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the Checkbox primitive — its checkbox role and checked state, the state a press asks for, its label
// ABOUTME: Pins the box's inks in both schemes: the green with an on-primary tick when checked, an outline edge that clears 3:1 when not

import React from 'react';
import { StyleSheet, Text } from 'react-native';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { Check } from 'lucide-react-native';
import { BOREAL_DARK, BOREAL_LIGHT } from '@pierre/shared-constants';

/** A rendered element, as the queries return it. */
type ReactTestInstance = ReturnType<typeof screen.getByTestId>;

// NativeWind's own hook needs a stub under jest; the resolved scheme comes
// from the persisted appearance preference, which each test writes.
jest.mock('nativewind', () => ({
  useColorScheme: () => ({ colorScheme: 'dark', setColorScheme: jest.fn() }),
}));

jest.mock('../../../services/api', () => ({
  userApi: { updateTheme: jest.fn().mockResolvedValue(undefined) },
}));

import { ThemeProvider, useTheme } from '../../../contexts/ThemeContext';
import { Checkbox } from '../Checkbox';

const APPEARANCE_KEY = 'pierre.appearance_pref';
const LABEL = 'I understand the risk and accept it';
const SCHEMES = ['light', 'dark'] as const;
type Scheme = (typeof SCHEMES)[number];
const TOKENS = { light: BOREAL_LIGHT, dark: BOREAL_DARK } as const;

/** WCAG relative luminance of a `#rrggbb` colour. */
function luminance(hex: string): number {
  const value = hex.replace('#', '');
  const channels = [0, 2, 4].map((offset) => {
    const c = parseInt(value.slice(offset, offset + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** Renders the resolved scheme, so a test can wait for storage to answer. */
function SchemeProbe() {
  return <Text testID="scheme-probe">{useTheme().scheme}</Text>;
}

async function renderIn(scheme: Scheme, checked: boolean) {
  await AsyncStorage.setItem(APPEARANCE_KEY, scheme);
  const view = render(
    <ThemeProvider>
      <SchemeProbe />
      <Checkbox checked={checked} onChange={jest.fn()} label={LABEL} testID="consent" />
    </ThemeProvider>,
  );
  await waitFor(() => expect(screen.getByTestId('scheme-probe')).toHaveTextContent(scheme));
  return view;
}

/** The box: the one view under the control that draws an edge. */
function box(): { backgroundColor: string; borderColor: string } {
  const control = screen.getByTestId('consent');
  const edged = control.findAll(
    (node: ReactTestInstance) =>
      typeof node.type === 'string' && StyleSheet.flatten(node.props.style)?.borderColor !== undefined,
  );
  expect(edged).toHaveLength(1);
  return StyleSheet.flatten(edged[0].props.style) as { backgroundColor: string; borderColor: string };
}

describe('Checkbox', () => {
  it('is a checkbox named by its label, reporting unchecked, with no tick', () => {
    const { UNSAFE_queryByType } = render(
      <Checkbox checked={false} onChange={jest.fn()} label={LABEL} testID="consent" />,
    );
    const control = screen.getByTestId('consent');
    expect(control.props.accessibilityRole).toBe('checkbox');
    expect(control.props.accessibilityState).toEqual({ checked: false });
    expect(control.props.accessibilityLabel).toBe(LABEL);
    expect(control).not.toBeChecked();
    expect(screen.getByText(LABEL)).toBeTruthy();
    expect(UNSAFE_queryByType(Check)).toBeNull();
  });

  it('reports checked and draws the tick when checked', () => {
    const { UNSAFE_getByType } = render(
      <Checkbox checked onChange={jest.fn()} label={LABEL} testID="consent" />,
    );
    const control = screen.getByTestId('consent');
    expect(control.props.accessibilityState).toEqual({ checked: true });
    expect(control).toBeChecked();
    expect(UNSAFE_getByType(Check)).toBeTruthy();
  });

  it('asks for the opposite state on a press', () => {
    const onChange = jest.fn();
    const { rerender } = render(
      <Checkbox checked={false} onChange={onChange} label={LABEL} testID="consent" />,
    );
    fireEvent.press(screen.getByTestId('consent'));
    expect(onChange).toHaveBeenLastCalledWith(true);

    rerender(<Checkbox checked onChange={onChange} label={LABEL} testID="consent" />);
    fireEvent.press(screen.getByTestId('consent'));
    expect(onChange).toHaveBeenLastCalledWith(false);
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it.each(SCHEMES)('fills the checked box with primary and ticks it in onPrimary, %s', async (scheme) => {
    const { UNSAFE_getByType } = await renderIn(scheme, true);
    expect(box()).toMatchObject({
      backgroundColor: TOKENS[scheme].primary,
      borderColor: TOKENS[scheme].primary,
    });
    expect(UNSAFE_getByType(Check).props.color).toBe(TOKENS[scheme].onPrimary);
    expect(contrast(TOKENS[scheme].onPrimary, TOKENS[scheme].primary)).toBeGreaterThanOrEqual(4.5);
  });

  it.each(SCHEMES)('edges the empty box in an ink that clears 3:1 on the canvas, %s', async (scheme) => {
    await renderIn(scheme, false);
    const { backgroundColor, borderColor } = box();
    expect(backgroundColor).toBe('transparent');
    expect(borderColor).toBe(TOKENS[scheme].outline);
    // The empty box is the only sign the control is there (WCAG 1.4.11).
    expect(contrast(borderColor, TOKENS[scheme].surface)).toBeGreaterThanOrEqual(3);
  });

  it('draws its label in the ink class it is handed', () => {
    render(
      <Checkbox
        checked={false}
        onChange={jest.fn()}
        label={LABEL}
        labelClassName="text-on-warning-container"
        testID="consent"
      />,
    );
    expect(screen.getByText(LABEL).props.className).toContain('text-on-warning-container');
  });
});
