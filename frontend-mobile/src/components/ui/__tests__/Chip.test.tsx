// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the Chip, SquareIconButton, StepperField and HeadlineInput primitives the day editor draws with
// ABOUTME: Pins each one's role, state and name for a screen reader, and the 44pt floor every one of them keeps

import React from 'react';
import { fireEvent, render, screen } from '@testing-library/react-native';
import { Chip } from '../Chip';
import { HeadlineInput } from '../HeadlineInput';
import { SquareIconButton } from '../SquareIconButton';
import { StepperField } from '../StepperField';

describe('Chip', () => {
  it('is a toggle button reporting whether it is selected, picked by fill, edge and weight', () => {
    const onPress = jest.fn();
    const { rerender } = render(<Chip label="Run" selected={false} onPress={onPress} testID="chip" />);
    const chip = screen.getByTestId('chip');
    expect(chip).toHaveProp('accessibilityRole', 'button');
    expect(chip).toHaveProp('accessibilityState', { selected: false });
    expect(chip).toHaveProp('accessibilityLabel', 'Run');
    expect(chip.props.className).toMatch(/\bmin-h-11\b/);
    // The phone's chip radius is 4 (DESIGN.md "Shapes"): never a pill.
    expect(chip.props.className).toMatch(/\brounded\b/);
    expect(chip.props.className).not.toMatch(/\brounded-full\b/);
    fireEvent.press(chip);
    expect(onPress).toHaveBeenCalledTimes(1);

    rerender(<Chip label="Run" selected onPress={onPress} testID="chip" />);
    expect(screen.getByTestId('chip')).toHaveProp('accessibilityState', { selected: true });
    expect(screen.getByTestId('chip').props.className).toMatch(/\bborder-primary\b/);
    expect(screen.getByText('Run').props.className).toMatch(/\bfont-semibold\b/);
  });

  it('is one radio of a group as a tile, named apart from a symbol label, dashed when it opens a field', () => {
    render(
      <>
        <Chip label="%" accessibilityLabel="% of a threshold" shape="tile" role="radio" selected onPress={jest.fn()} testID="pct" />
        <Chip label="Other…" selected={false} dashed onPress={jest.fn()} testID="other" />
      </>,
    );
    const tile = screen.getByTestId('pct');
    expect(tile).toHaveProp('accessibilityRole', 'radio');
    expect(tile).toHaveProp('accessibilityState', { checked: true });
    expect(tile).toHaveProp('accessibilityLabel', '% of a threshold');
    expect(tile.props.className).toMatch(/\bmin-w-11\b/);
    expect(screen.getByText('%').props.className).toMatch(/\bfont-mono\b/);
    expect(screen.getByTestId('other').props.className).toMatch(/\bborder-dashed\b/);
  });
});

describe('SquareIconButton', () => {
  it('is a 44pt button named by its label, inert when disabled', () => {
    const onPress = jest.fn();
    render(<SquareIconButton icon="plus" label="One more repetition" disabled onPress={onPress} testID="more" />);
    const button = screen.getByTestId('more');
    expect(button).toHaveProp('accessibilityLabel', 'One more repetition');
    expect(button).toHaveProp('accessibilityState', { disabled: true });
    expect(button.props.className).toMatch(/\bh-11\b/);
    expect(button.props.className).toMatch(/\bw-11\b/);
    fireEvent.press(button);
    expect(onPress).not.toHaveBeenCalled();
  });
});

describe('StepperField', () => {
  it('flanks its field with the buttons it is given, and stands alone without them', () => {
    const less = jest.fn();
    const more = jest.fn();
    const { rerender } = render(
      <StepperField
        value="3:00"
        onChangeText={jest.fn()}
        accessibilityLabel="Duration (m:ss)"
        decrement={{ label: '−15s', onPress: less, testID: 'less' }}
        increment={{ label: '+15s', onPress: more, testID: 'more' }}
        testID="field"
      />,
    );
    expect(screen.getByTestId('field')).toHaveProp('accessibilityLabel', 'Duration (m:ss)');
    expect(screen.getByTestId('field').props.className).toMatch(/\bh-11\b/);
    fireEvent.press(screen.getByTestId('less'));
    fireEvent.press(screen.getByTestId('more'));
    expect(less).toHaveBeenCalledTimes(1);
    expect(more).toHaveBeenCalledTimes(1);

    rerender(<StepperField value="3:00" onChangeText={jest.fn()} accessibilityLabel="Estimated time" testID="field" />);
    expect(screen.queryByTestId('less')).toBeNull();
    expect(screen.queryByTestId('more')).toBeNull();
  });
});

describe('HeadlineInput', () => {
  it('is a field in the display face, named by its label since none is drawn', () => {
    const onChangeText = jest.fn();
    render(<HeadlineInput value="" placeholder="Session title" accessibilityLabel="Title" onChangeText={onChangeText} testID="title" />);
    const field = screen.getByTestId('title');
    expect(field).toHaveProp('accessibilityLabel', 'Title');
    expect(field.props.className).toMatch(/\bfont-display\b/);
    expect(field.props.className).toMatch(/\bmin-h-11\b/);
    fireEvent.changeText(field, 'Tempo');
    expect(onChangeText).toHaveBeenCalledWith('Tempo');
  });
});
