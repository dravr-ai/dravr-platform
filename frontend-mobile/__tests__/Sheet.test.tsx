// ABOUTME: Pins the one bottom sheet — the scrim class, the 20 radius, the two testIDs, and no drag pill anywhere
// ABOUTME: Behaviour: children render only while visible, and a press on the scrim closes it (Boreal v2.2 P3.6)

import React from 'react';
import { Text, TouchableOpacity } from 'react-native';
import { fireEvent, render } from '@testing-library/react-native';
import { Sheet } from '../src/components/ui/Sheet';

type ClassNamed = { props: { className?: string }; children?: unknown };

/** Every className in the rendered tree, host views included. */
function classNames(node: unknown): string[] {
  if (!node || typeof node !== 'object') return [];
  const found: string[] = [];
  const el = node as ClassNamed;
  if (el.props && typeof el.props.className === 'string') found.push(el.props.className);
  const children = Array.isArray(node) ? node : el.children;
  if (Array.isArray(children)) {
    for (const child of children) found.push(...classNames(child));
  }
  return found;
}

function renderSheet(visible = true) {
  const onClose = jest.fn();
  const view = render(
    <Sheet visible={visible} onClose={onClose} testID="the-sheet" backdropTestID="the-backdrop">
      <Text>Inside the sheet</Text>
    </Sheet>,
  );
  return { ...view, onClose };
}

describe('Sheet', () => {
  it('renders its children while visible', () => {
    const { getByText } = renderSheet();
    expect(getByText('Inside the sheet')).toBeTruthy();
  });

  it('renders nothing while hidden', () => {
    const { queryByText, queryByTestId } = renderSheet(false);
    expect(queryByText('Inside the sheet')).toBeNull();
    expect(queryByTestId('the-sheet')).toBeNull();
  });

  it('closes on a press of the scrim', () => {
    const { getByTestId, onClose } = renderSheet();
    fireEvent.press(getByTestId('the-backdrop'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('carries testID on the panel and backdropTestID on the scrim', () => {
    const { getByTestId, UNSAFE_getByType } = renderSheet();
    const panel = getByTestId('the-sheet');
    expect(panel.props.className).toContain('rounded-t-3xl');
    expect(panel.props.className).toContain('max-h-[85%]');
    expect(panel.props.onStartShouldSetResponder()).toBe(true);
    // The touchable's host view does not carry the className; the element does.
    const backdrop = UNSAFE_getByType(TouchableOpacity);
    expect(backdrop.props.testID).toBe('the-backdrop');
    expect(getByTestId('the-backdrop')).toBeTruthy();
    expect(backdrop.props.className).toContain('bg-scrim/60');
    expect(backdrop.props.className).toContain('justify-end');
  });

  it('paints the panel on the secondary ground, through the token', () => {
    const { getByTestId } = renderSheet();
    const styles = ([] as Array<Record<string, unknown>>).concat(getByTestId('the-sheet').props.style);
    expect(styles.some((s) => typeof s?.backgroundColor === 'string')).toBe(true);
  });

  it('honours a caller-supplied max height class', () => {
    const { getByTestId } = render(
      <Sheet visible onClose={() => {}} testID="tall" maxHeight="max-h-[95%]">
        <Text>Tall</Text>
      </Sheet>,
    );
    expect(getByTestId('tall').props.className).toContain('max-h-[95%]');
    expect(getByTestId('tall').props.className).not.toContain('max-h-[85%]');
  });

  it('pads the panel 16 on the sides unless the caller asks for flush content', () => {
    const padded = render(
      <Sheet visible onClose={() => {}} testID="padded">
        <Text>Padded</Text>
      </Sheet>,
    );
    expect(padded.getByTestId('padded').props.className).toContain('px-4');

    const flush = render(
      <Sheet visible onClose={() => {}} testID="flush" flush>
        <Text>Flush</Text>
      </Sheet>,
    );
    expect(flush.getByTestId('flush').props.className).not.toContain('px-4');
    expect(flush.getByTestId('flush').props.className).toContain('pt-4');
  });

  it('has no drag pill anywhere in the tree', () => {
    const { toJSON } = renderSheet();
    const all = classNames(toJSON());
    expect(all.length).toBeGreaterThan(0);
    expect(all.some((c) => /\bw-9\b/.test(c) && /\bh-1\b/.test(c))).toBe(false);
  });
});
