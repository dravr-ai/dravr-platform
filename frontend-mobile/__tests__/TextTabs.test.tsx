// ABOUTME: Pins the text tabs — 13 / 500 labels 20 apart over a hairline, exactly one selected, the primary underline on that one only
// ABOUTME: Behaviour: a press on another tab reports its key, and `${testID}-${key}` reaches every tab

import React from 'react';
import { ScrollView, StyleSheet } from 'react-native';
import { fireEvent, render } from '@testing-library/react-native';
import { TextTabs } from '../src/components/ui/TextTabs';

const items = [
  { key: 'all', label: 'All' },
  { key: 'goal', label: 'Goals' },
  { key: 'preference', label: 'Preferences' },
];

type Json = { type: string; props: Record<string, unknown>; children: Array<Json | string> | null };

/** The host node carrying `testID` in a rendered tree. */
function findByTestId(node: Json | string | null | undefined, testID: string): Json | undefined {
  if (!node || typeof node === 'string') return undefined;
  if (node.props.testID === testID) return node;
  for (const child of node.children ?? []) {
    const hit = findByTestId(child, testID);
    if (hit) return hit;
  }
  return undefined;
}

/** The underline is the tab's last host child: the view under the label. */
function underlineOf(tree: ReturnType<typeof render>['toJSON'], testID: string): Json {
  const rendered = tree();
  const tab = findByTestId(Array.isArray(rendered) ? rendered[0] : rendered, testID);
  if (!tab) throw new Error(`no tab ${testID}`);
  const hosts = (tab.children ?? []).filter((c): c is Json => typeof c !== 'string');
  return hosts[hosts.length - 1];
}

describe('TextTabs', () => {
  it('selects exactly one tab and marks it for assistive tech', () => {
    const { getAllByRole } = render(<TextTabs items={items} value="goal" onChange={() => {}} />);
    const tabs = getAllByRole('tab');
    expect(tabs).toHaveLength(3);
    const selected = tabs.filter((tab) => tab.props.accessibilityState?.selected === true);
    expect(selected).toHaveLength(1);
    expect(selected[0].props.testID).toBeUndefined();
    expect(tabs.filter((tab) => tab.props.accessibilityState?.selected === false)).toHaveLength(2);
  });

  it('inks the active label primary with a 2 px primary underline, the others secondary over a transparent one', () => {
    const { toJSON, getByText } = render(
      <TextTabs items={items} value="goal" onChange={() => {}} testID="memory-kind" />,
    );
    const active = getByText('Goals');
    expect(active.props.className).toContain('text-sm');
    expect(active.props.className).toContain('font-medium');
    expect(active.props.className).toContain('text-text-primary');
    const activeUnderline = underlineOf(toJSON, 'memory-kind-goal');
    expect(activeUnderline.props.className).toContain('h-0.5');
    expect(activeUnderline.props.className).toContain('bg-primary');

    const inactive = getByText('All');
    expect(inactive.props.className).toContain('text-text-secondary');
    expect(inactive.props.className).not.toContain('text-text-primary');
    const inactiveUnderline = underlineOf(toJSON, 'memory-kind-all');
    expect(inactiveUnderline.props.className).toContain('h-0.5');
    expect(inactiveUnderline.props.className).toContain('bg-transparent');
    expect(inactiveUnderline.props.className).not.toContain('bg-primary');
  });

  it('reports the pressed tab by its key', () => {
    const onChange = jest.fn();
    const { getByTestId } = render(
      <TextTabs items={items} value="all" onChange={onChange} testID="memory-kind" />,
    );
    fireEvent.press(getByTestId('memory-kind-preference'));
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith('preference');
  });

  it('forwards `${testID}-${key}` to every tab and testID to the row', () => {
    const { getByTestId } = render(
      <TextTabs items={items} value="all" onChange={() => {}} testID="memory-kind" />,
    );
    expect(getByTestId('memory-kind')).toBeTruthy();
    for (const item of items) {
      expect(getByTestId(`memory-kind-${item.key}`).props.accessibilityRole).toBe('tab');
    }
  });

  it('lays the labels 20 apart on one horizontal scroll row over a hairline', () => {
    const { getByTestId, UNSAFE_getByType } = render(
      <TextTabs items={items} value="all" onChange={() => {}} testID="memory-kind" className="mb-2" />,
    );
    const row = getByTestId('memory-kind');
    expect(row.props.className).toContain('border-b');
    expect(row.props.className).toContain('border-border');
    expect(row.props.className).toContain('mb-2');
    const rowStyles = ([] as Array<Record<string, unknown>>).concat(row.props.style);
    expect(rowStyles.some((s) => s?.borderBottomWidth === StyleSheet.hairlineWidth)).toBe(true);
    const scroll = UNSAFE_getByType(ScrollView);
    expect(scroll.props.horizontal).toBe(true);
    expect(scroll.props.showsHorizontalScrollIndicator).toBe(false);
    expect(scroll.props.contentContainerClassName).toContain('gap-5');
    expect(scroll.props.contentContainerClassName).toContain('flex-row');
    expect(getByTestId('memory-kind-all').props.className).toContain('py-2.5');
  });
});
