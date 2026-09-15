// ABOUTME: Pins the settings row — 52 tall (44 compact), title and hint on one inner row, the faint hairline on that inner column, none on the last
// ABOUTME: And its trailing grammar: a chevron only when it presses and nothing else sits there, the value slot in mono, press and long-press firing

import React from 'react';
import { StyleSheet, Text, View } from 'react-native';
import { fireEvent, render } from '@testing-library/react-native';
import { Row } from '../src/components/ui/Row';

// The glyph as a text node carrying its name and ink, so a spec can find the
// chevron by test id and read the colour it was handed.
jest.mock('@expo/vector-icons', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return {
    Feather: ({ name, color, size }: { name: string; color: string; size: number }) =>
      React.createElement(Text, { testID: `icon-${name}`, style: { color, fontSize: size } }, name),
  };
});

type Styled = { props: { style?: unknown } };
type Instance = { type: unknown; parent: Instance | null; props: { className?: string } };

/** The nearest host view above an element — `.parent` alone lands on the composite `Text` wrapper. */
function hostParent(el: Instance): Instance {
  let node = el.parent;
  while (node && typeof node.type !== 'string') node = node.parent;
  if (!node) throw new Error('no host parent');
  return node;
}

/** Every style object on an element, whether given flat or as an array. */
function styles(el: Styled): Array<Record<string, unknown>> {
  return ([] as Array<Record<string, unknown>>).concat(
    (el.props.style as Array<Record<string, unknown>> | Record<string, unknown> | undefined) ?? [],
  );
}

describe('Row', () => {
  it('is 52 tall by default and 44 when compact', () => {
    const tall = render(<Row title="Account" testID="row" />);
    expect(tall.getByTestId('row-inner').props.className).toContain('min-h-[52px]');
    expect(tall.getByTestId('row-inner').props.className).toContain('flex-row');
    expect(tall.getByTestId('row-inner').props.className).toContain('items-center');
    const compact = render(<Row title="Version" compact testID="row" />);
    expect(compact.getByTestId('row-inner').props.className).toContain('min-h-[44px]');
    expect(compact.getByTestId('row-inner').props.className).not.toContain('min-h-[52px]');
  });

  it('sets the title at 16 in the primary ink and the hint at 13 tertiary, both inside the inner row', () => {
    const { getByText, getByTestId } = render(<Row title="Language" hint="Français" testID="row" />);
    const title = getByText('Language');
    expect(title.props.className).toContain('text-base');
    expect(title.props.className).toContain('text-text-primary');
    const hint = getByText('Français');
    expect(hint.props.className).toContain('text-sm');
    expect(hint.props.className).toContain('text-text-tertiary');
    expect(hint.props.className).toContain('shrink');
    expect(hint.props.className).toContain('text-right');
    expect(hint.props.numberOfLines).toBe(1);
    expect(hint.props.ellipsizeMode).toBe('tail');
    // With a hint the title block does not shrink and the trailing group takes
    // the rest of the row, so the hint is the line that truncates — never the
    // pane's name, which wrapped "Data providers" onto three lines when the
    // title block was the flexible one.
    const titleBlock = hostParent(title);
    expect(titleBlock.props.className).toContain('shrink-0');
    expect(titleBlock.props.className).not.toContain('flex-1');
    expect(hostParent(titleBlock)).toBe(getByTestId('row-inner'));
    const trailingGroup = hostParent(hint);
    expect(trailingGroup.props.className).toContain('flex-1');
    expect(trailingGroup.props.className).toContain('min-w-0');
    expect(trailingGroup.props.className).toContain('justify-end');
    expect(trailingGroup.props.className).toContain('ml-3');
    expect(hostParent(trailingGroup)).toBe(getByTestId('row-inner'));
  });

  it('lets the title block take the row and wrap when there is no hint', () => {
    const { getByText } = render(<Row title="Casual" subtitle="Short answers, no jargon." testID="row" />);
    const titleBlock = hostParent(getByText('Casual'));
    expect(titleBlock.props.className).toContain('flex-1');
    expect(titleBlock.props.className).toContain('min-w-0');
  });

  it('lands hintTestID on the hint text, so a spec reads the state by id', () => {
    const { getByTestId, getByText } = render(
      <Row title="Model" hint="Copilot Headless · claude-sonnet-5" hintTestID="model-value" />,
    );
    const hint = getByTestId('model-value');
    expect(hint).toBe(getByText('Copilot Headless · claude-sonnet-5'));
    expect(hint.props.children).toBe('Copilot Headless · claude-sonnet-5');
    expect(hint.props.className).toContain('text-text-tertiary');
  });

  it('renders the subtitle as a second line under the title', () => {
    const { getByText } = render(<Row title="Strava" subtitle="Connected yesterday" />);
    const subtitle = getByText('Connected yesterday');
    expect(subtitle.props.className).toContain('text-sm');
    expect(subtitle.props.className).toContain('text-text-secondary');
    expect(hostParent(subtitle)).toBe(hostParent(getByText('Strava')));
  });

  it('draws the faint hairline on the inner column at hairline width, and none on the last row', () => {
    const middle = render(<Row title="Account" testID="row" />);
    const inner = middle.getByTestId('row-inner');
    expect(inner.props.className).toContain('border-b');
    expect(inner.props.className).toContain('border-border-faint');
    expect(styles(inner).some((s) => s.borderBottomWidth === StyleSheet.hairlineWidth)).toBe(true);
    // The outer element pays the inset and carries no line of its own.
    expect(middle.getByTestId('row').props.className).toBe('px-4');

    const last = render(<Row title="About" last testID="row" />);
    const lastInner = last.getByTestId('row-inner');
    expect(lastInner.props.className).not.toContain('border');
    expect(styles(lastInner).some((s) => s.borderBottomWidth !== undefined)).toBe(false);
  });

  it('shows the chevron when it presses and nothing else sits on the trailing side', () => {
    const { getByTestId, queryByTestId } = render(<Row title="Account" onPress={() => {}} testID="row" />);
    const chevron = getByTestId('icon-chevron-right');
    expect(chevron.props.style.fontSize).toBe(18);
    expect(typeof chevron.props.style.color).toBe('string');
    expect(queryByTestId('icon-chevron-down')).toBeNull();
  });

  it('shows no chevron when a trailing node or a value takes that side, or when the row does not press', () => {
    const withTrailing = render(
      <Row title="Notifications" onPress={() => {}} trailing={<View testID="switch" />} />,
    );
    expect(withTrailing.getByTestId('switch')).toBeTruthy();
    expect(withTrailing.queryByTestId('icon-chevron-right')).toBeNull();

    const withValue = render(<Row title="Tokens" onPress={() => {}} value="1,204" />);
    expect(withValue.queryByTestId('icon-chevron-right')).toBeNull();

    const still = render(<Row title="Version" hint="2.2.0" />);
    expect(still.queryByTestId('icon-chevron-right')).toBeNull();
  });

  it('honours an explicit showChevron either way', () => {
    const forced = render(<Row title="Tokens" onPress={() => {}} value="1,204" showChevron />);
    expect(forced.getByTestId('icon-chevron-right')).toBeTruthy();
    const suppressed = render(<Row title="Account" onPress={() => {}} showChevron={false} />);
    expect(suppressed.queryByTestId('icon-chevron-right')).toBeNull();
  });

  it('fires onPress and onLongPress, and reads as a button', () => {
    const onPress = jest.fn();
    const onLongPress = jest.fn();
    const { getByTestId } = render(
      <Row title="Account" onPress={onPress} onLongPress={onLongPress} testID="row" />,
    );
    fireEvent.press(getByTestId('row'));
    expect(onPress).toHaveBeenCalledTimes(1);
    fireEvent(getByTestId('row'), 'longPress');
    expect(onLongPress).toHaveBeenCalledTimes(1);
    expect(getByTestId('row').props.accessibilityRole).toBe('button');
  });

  it('is a plain view with no button role when nothing presses it', () => {
    const { getByTestId } = render(<Row title="Version" hint="2.2.0" testID="row" />);
    const row = getByTestId('row');
    expect(row.props.accessibilityRole).toBeUndefined();
    // A plain View claims no touch: a Pressable's host would carry the responder handlers.
    expect(row.props.onStartShouldSetResponder).toBeUndefined();
    expect(row.props.onResponderGrant).toBeUndefined();
  });

  it('forwards the accessibility props over the defaults', () => {
    const { getByTestId } = render(
      <Row
        title="Dark mode"
        onPress={() => {}}
        accessibilityRole="switch"
        accessibilityState={{ checked: true }}
        accessibilityLabel="Dark mode, on"
        testID="row"
      />,
    );
    const row = getByTestId('row');
    expect(row.props.accessibilityRole).toBe('switch');
    expect(row.props.accessibilityState).toEqual(expect.objectContaining({ checked: true }));
    expect(row.props.accessibilityLabel).toBe('Dark mode, on');
  });

  it('sets the value in mono with tabular figures on the secondary ink', () => {
    const { getByText } = render(<Row title="Tokens this month" value="1,204" />);
    const value = getByText('1,204');
    expect(value.props.className).toContain('font-mono');
    expect(value.props.className).toContain('tabular-nums');
    expect(value.props.className).toContain('text-sm');
    expect(value.props.className).toContain('text-text-secondary');
  });
});
