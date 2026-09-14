// ABOUTME: Pins the empty state — one interface-size sentence and, when given, one inline ink link that presses
// ABOUTME: Mirrors the web's EmptyState API on React Native: sentence + action, no icon, no card, no filled button

import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';
import { EmptyState } from '../src/components/ui/EmptyState';

describe('EmptyState', () => {
  it('renders the sentence at the interface size in the secondary ink', () => {
    const { getByText } = render(<EmptyState>No conversations yet.</EmptyState>);
    const sentence = getByText('No conversations yet.');
    expect(sentence.props.className).toContain('text-sm');
    expect(sentence.props.className).toContain('text-text-secondary');
  });

  it('renders the action as an inline ink link that presses', () => {
    const onPress = jest.fn();
    const { getByTestId, getByText } = render(
      <EmptyState action={{ label: 'Start a discussion', onPress, testID: 'empty-start' }}>
        No conversations yet.
      </EmptyState>,
    );
    const link = getByTestId('empty-start');
    expect(link.props.className).toContain('text-primary');
    expect(link.props.className).toContain('font-medium');
    expect(link.props.accessibilityRole).toBe('button');
    expect(getByText('Start a discussion')).toBeTruthy();
    fireEvent.press(link);
    expect(onPress).toHaveBeenCalledTimes(1);
  });

  it('sets the link beside the sentence on one wrapping row, as its own view', () => {
    const { getByText, getByTestId } = render(
      <EmptyState testID="empty" action={{ label: 'Start a discussion', onPress: () => {}, testID: 'empty-start' }}>
        No conversations yet.
      </EmptyState>,
    );
    // Two Text nodes on a flex-wrap row: Android gives a nested Text no
    // native view, so a sibling is what a test id and a tap can reach there.
    expect(getByText('No conversations yet.').props.className).toContain('text-sm');
    expect(getByText('Start a discussion').props.className).toContain('text-sm');
    expect(getByTestId('empty').props.className).toContain('flex-row');
    expect(getByTestId('empty').props.className).toContain('flex-wrap');
    expect(getByTestId('empty-start').parent?.parent).toBe(getByTestId('empty'));
  });

  it('renders no link when there is no action', () => {
    const { queryByRole, toJSON } = render(
      <EmptyState testID="empty">Nothing here.</EmptyState>,
    );
    expect(queryByRole('button')).toBeNull();
    expect(JSON.stringify(toJSON())).not.toContain('text-primary');
  });

  it('passes testID and extra classes to the wrapping view', () => {
    const { getByTestId } = render(
      <EmptyState testID="empty" className="pt-6">
        Nothing here.
      </EmptyState>,
    );
    const wrapper = getByTestId('empty');
    expect(wrapper.props.className).toContain('px-4');
    expect(wrapper.props.className).toContain('py-3');
    expect(wrapper.props.className).toContain('pt-6');
  });
});
