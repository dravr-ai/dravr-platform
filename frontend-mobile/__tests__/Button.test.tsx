// ABOUTME: Unit tests for Button component
// ABOUTME: Tests the four variants, the 44/8/no-shadow shape, states, and interactions

import React from 'react';
import type { ViewStyle } from 'react-native';
import { render, fireEvent } from '@testing-library/react-native';
import { BOREAL_DARK } from '@pierre/shared-constants';
import { Button } from '../src/components/ui/Button';

describe('Button Component', () => {
  describe('rendering', () => {
    it('should render with title', () => {
      const { getByText } = render(
        <Button title="Click me" onPress={() => {}} />
      );
      expect(getByText('Click me')).toBeTruthy();
    });

    it('should render with testID', () => {
      const { getByTestId } = render(
        <Button title="Test" onPress={() => {}} testID="test-button" />
      );
      expect(getByTestId('test-button')).toBeTruthy();
    });
  });

  describe('variants', () => {
    it('should render primary variant by default', () => {
      const { getByText } = render(
        <Button title="Primary" onPress={() => {}} />
      );
      expect(getByText('Primary')).toBeTruthy();
    });

    it('should render secondary variant', () => {
      const { getByText } = render(
        <Button title="Secondary" onPress={() => {}} variant="secondary" />
      );
      expect(getByText('Secondary')).toBeTruthy();
    });

    it('should render ghost variant', () => {
      const { getByText } = render(
        <Button title="Ghost" onPress={() => {}} variant="ghost" />
      );
      expect(getByText('Ghost')).toBeTruthy();
    });

    it('should render danger variant', () => {
      const { getByText } = render(
        <Button title="Danger" onPress={() => {}} variant="danger" />
      );
      expect(getByText('Danger')).toBeTruthy();
    });
  });

  describe('shape (Boreal v2.2 D4)', () => {
    it('is 44 tall with radius 8 and casts no shadow at rest', () => {
      const { UNSAFE_getByType } = render(
        <Button title="Shape" onPress={() => {}} testID="shape-button" />
      );
      // The touchable's host view does not carry the className; the element does.
      const TouchableOpacity = require('react-native').TouchableOpacity;
      const button = UNSAFE_getByType(TouchableOpacity);
      const className = button.props.className as string;
      expect(className).toContain('h-11');
      expect(className).toContain('rounded-lg');
      expect(className).not.toMatch(/rounded-(xl|full)/);
      const style = button.props.style as ViewStyle | undefined;
      expect(style?.shadowOpacity).toBeUndefined();
      expect(style?.elevation).toBeUndefined();
    });

    it('spins in the label ink of the ground, not a frozen hex', () => {
      const ActivityIndicator = require('react-native').ActivityIndicator;
      const filled = render(<Button title="Go" onPress={() => {}} loading />);
      expect(filled.UNSAFE_getByType(ActivityIndicator).props.color).toBe(BOREAL_DARK.onPrimary);
      const bare = render(<Button title="Go" onPress={() => {}} loading variant="ghost" />);
      expect(bare.UNSAFE_getByType(ActivityIndicator).props.color).toBe(BOREAL_DARK.primary);
    });
  });

  describe('interactions', () => {
    it('should call onPress when pressed', () => {
      const onPressMock = jest.fn();
      const { getByText } = render(
        <Button title="Click me" onPress={onPressMock} />
      );

      fireEvent.press(getByText('Click me'));
      expect(onPressMock).toHaveBeenCalledTimes(1);
    });

    it('should not call onPress when disabled', () => {
      const onPressMock = jest.fn();
      const { getByText } = render(
        <Button title="Disabled" onPress={onPressMock} disabled />
      );

      fireEvent.press(getByText('Disabled'));
      expect(onPressMock).not.toHaveBeenCalled();
    });
  });

  describe('loading state', () => {
    it('should show loading indicator when loading', () => {
      const { queryByText, UNSAFE_getByType } = render(
        <Button title="Loading" onPress={() => {}} loading />
      );

      // Text should not be visible when loading
      expect(queryByText('Loading')).toBeNull();
      // ActivityIndicator should be rendered
      const ActivityIndicator = require('react-native').ActivityIndicator;
      expect(UNSAFE_getByType(ActivityIndicator)).toBeTruthy();
    });

    it('should not call onPress when loading', () => {
      const onPressMock = jest.fn();
      const { getByTestId } = render(
        <Button title="Loading" onPress={onPressMock} loading testID="loading-btn" />
      );

      fireEvent.press(getByTestId('loading-btn'));
      expect(onPressMock).not.toHaveBeenCalled();
    });
  });

  describe('fullWidth prop', () => {
    it('should accept fullWidth prop', () => {
      const { getByText } = render(
        <Button title="Full Width" onPress={() => {}} fullWidth />
      );
      expect(getByText('Full Width')).toBeTruthy();
    });
  });

  describe('custom styles', () => {
    it('should accept custom style prop', () => {
      const { getByText } = render(
        <Button
          title="Styled"
          onPress={() => {}}
          style={{ marginTop: 10 }}
        />
      );
      expect(getByText('Styled')).toBeTruthy();
    });

    it('should accept custom textStyle prop', () => {
      const { getByText } = render(
        <Button
          title="Styled Text"
          onPress={() => {}}
          textStyle={{ letterSpacing: 2 }}
        />
      );
      expect(getByText('Styled Text')).toBeTruthy();
    });
  });
});
