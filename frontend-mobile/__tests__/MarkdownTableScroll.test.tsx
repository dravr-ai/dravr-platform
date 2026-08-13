// ABOUTME: Tests the markdown table render-rule override used by chat messages
// ABOUTME: A coach table wider than the phone must scroll horizontally, not clip

import React from 'react';
import { render } from '@testing-library/react-native';
import Markdown from 'react-native-markdown-display';
import { MARKDOWN_RULES } from '../src/screens/chat/markdownRules';

// A realistic coach reply: a training week with enough columns that the table
// cannot fit a phone width. This is the shape that motivated the override — the
// library lays a table out in a plain View, so the overflow was unreachable.
const WIDE_TABLE = [
  '| Day | Session | Distance | Pace | Elevation |',
  '| --- | --- | --- | --- | --- |',
  '| Tuesday | Threshold | 12 km | 4:35/km | 120 m |',
  '| Thursday | Easy | 8 km | 5:40/km | 40 m |',
  '| Sunday | Long run | 24 km | 5:15/km | 460 m |',
].join('\n');

describe('MARKDOWN_RULES table override', () => {
  it('renders every cell of a wide table', () => {
    const { getByText } = render(
      <Markdown rules={MARKDOWN_RULES}>{WIDE_TABLE}</Markdown>
    );

    // Header cells survive the override.
    expect(getByText('Day')).toBeTruthy();
    expect(getByText('Elevation')).toBeTruthy();

    // Body cells across the full column span — a table that clipped or
    // collapsed would drop the rightmost column.
    expect(getByText('Tuesday')).toBeTruthy();
    expect(getByText('12 km')).toBeTruthy();
    expect(getByText('460 m')).toBeTruthy();
  });

  it('wraps the table in a horizontally scrollable container', () => {
    const { getByTestId } = render(
      <Markdown rules={MARKDOWN_RULES}>{WIDE_TABLE}</Markdown>
    );

    const scroller = getByTestId('markdown-table-scroll');
    // `horizontal` is the whole point — a vertical ScrollView would still clip
    // the rightmost columns, so assert the prop rather than mere presence.
    expect(scroller.props.horizontal).toBe(true);
  });

  it('does not wrap prose that contains no table', () => {
    const { queryByTestId, getByText } = render(
      <Markdown rules={MARKDOWN_RULES}>
        {'Ta charge grimpe depuis trois semaines.'}
      </Markdown>
    );

    expect(getByText('Ta charge grimpe depuis trois semaines.')).toBeTruthy();
    expect(queryByTestId('markdown-table-scroll')).toBeNull();
  });

  it('leaves non-table markdown rules intact', () => {
    const { getByText, queryByTestId } = render(
      <Markdown rules={MARKDOWN_RULES}>
        {'# Semaine 3\n\n- Seuil mardi\n- Sortie longue dimanche'}
      </Markdown>
    );

    // Overriding `table` must not displace the library's other rules.
    expect(getByText('Semaine 3')).toBeTruthy();
    expect(getByText('Seuil mardi')).toBeTruthy();
    expect(queryByTestId('markdown-table-scroll')).toBeNull();
  });
});
