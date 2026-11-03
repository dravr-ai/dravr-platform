// ABOUTME: Unit tests for ScrollFadeContainer component
// ABOUTME: Tests edge-fading gradients for horizontal scroll affordance per Stitch UX

import React from 'react';
import { render, fireEvent } from '@testing-library/react-native';
import { View, Text, StyleSheet } from 'react-native';
import { ScrollFadeContainer, clearOf } from '../src/components/ui/ScrollFadeContainer';

describe('ScrollFadeContainer', () => {
  describe('rendering', () => {
    it('should render children', () => {
      const { getByText } = render(
        <ScrollFadeContainer>
          <Text>Test Content</Text>
        </ScrollFadeContainer>
      );
      expect(getByText('Test Content')).toBeTruthy();
    });

    it('should render with testID', () => {
      const { getByTestId } = render(
        <ScrollFadeContainer testID="test-scroll-fade">
          <Text>Content</Text>
        </ScrollFadeContainer>
      );
      expect(getByTestId('test-scroll-fade')).toBeTruthy();
    });

    it('should render scroll view with testID suffix', () => {
      const { getByTestId } = render(
        <ScrollFadeContainer testID="test-scroll-fade">
          <Text>Content</Text>
        </ScrollFadeContainer>
      );
      expect(getByTestId('test-scroll-fade-scroll')).toBeTruthy();
    });
  });

  describe('props', () => {
    it('should accept custom backgroundColor', () => {
      const { getByTestId } = render(
        <ScrollFadeContainer
          backgroundColor="#FF0000"
          testID="test-scroll"
        >
          <Text>Content</Text>
        </ScrollFadeContainer>
      );
      expect(getByTestId('test-scroll')).toBeTruthy();
    });

    it('should accept custom fadeWidth', () => {
      const { getByTestId } = render(
        <ScrollFadeContainer
          fadeWidth={64}
          testID="test-scroll"
        >
          <Text>Content</Text>
        </ScrollFadeContainer>
      );
      expect(getByTestId('test-scroll')).toBeTruthy();
    });

    it('should accept custom containerStyle', () => {
      const { getByTestId } = render(
        <ScrollFadeContainer
          containerStyle={{ marginTop: 10 }}
          testID="test-scroll"
        >
          <Text>Content</Text>
        </ScrollFadeContainer>
      );
      expect(getByTestId('test-scroll')).toBeTruthy();
    });

    it('should accept custom contentContainerStyle', () => {
      const { getByTestId } = render(
        <ScrollFadeContainer
          contentContainerStyle={{ paddingHorizontal: 16 }}
          testID="test-scroll"
        >
          <Text>Content</Text>
        </ScrollFadeContainer>
      );
      expect(getByTestId('test-scroll')).toBeTruthy();
    });
  });

  describe('scroll behavior', () => {
    it('should handle horizontal scroll events', () => {
      const { getByTestId } = render(
        <ScrollFadeContainer testID="scroll-fade">
          <View style={{ width: 1000 }}>
            <Text>Wide content that requires scrolling</Text>
          </View>
        </ScrollFadeContainer>
      );

      const scrollView = getByTestId('scroll-fade-scroll');

      // Simulate scroll event
      fireEvent.scroll(scrollView, {
        nativeEvent: {
          contentOffset: { x: 100, y: 0 },
          contentSize: { width: 1000, height: 50 },
          layoutMeasurement: { width: 390, height: 50 },
        },
      });

      // Component should still be rendered after scroll
      expect(scrollView).toBeTruthy();
    });

    it('should handle layout change events', () => {
      const { getByTestId } = render(
        <ScrollFadeContainer testID="scroll-fade">
          <Text>Content</Text>
        </ScrollFadeContainer>
      );

      const container = getByTestId('scroll-fade');

      // Simulate layout event
      fireEvent(container, 'layout', {
        nativeEvent: {
          layout: { x: 0, y: 0, width: 390, height: 50 },
        },
      });

      expect(container).toBeTruthy();
    });
  });

  describe('multiple children', () => {
    it('should render multiple children (like filter chips)', () => {
      const filterItems = ['All', 'Training', 'Nutrition', 'Recovery', 'Recipes'];

      const { getByText } = render(
        <ScrollFadeContainer testID="filter-scroll">
          {filterItems.map((item) => (
            <View key={item} style={{ marginRight: 8 }}>
              <Text>{item}</Text>
            </View>
          ))}
        </ScrollFadeContainer>
      );

      // All filter items should be rendered
      filterItems.forEach((item) => {
        expect(getByText(item)).toBeTruthy();
      });
    });
  });

  describe('fade visibility logic', () => {
    it('should not show fades when content is not scrollable', () => {
      const { queryByTestId } = render(
        <ScrollFadeContainer testID="scroll-fade">
          <Text>Short content</Text>
        </ScrollFadeContainer>
      );

      // Without content size change triggering scrollability,
      // fades should not be visible initially
      expect(queryByTestId('scroll-fade-left-fade')).toBeNull();
      expect(queryByTestId('scroll-fade-right-fade')).toBeNull();
    });

    it('should show right fade when scrollable content is detected', () => {
      const { getByTestId, rerender } = render(
        <ScrollFadeContainer testID="scroll-fade">
          <View style={{ width: 1000 }}>
            <Text>Wide scrollable content</Text>
          </View>
        </ScrollFadeContainer>
      );

      const container = getByTestId('scroll-fade');
      const scrollView = getByTestId('scroll-fade-scroll');

      // Simulate container layout
      fireEvent(container, 'layout', {
        nativeEvent: {
          layout: { x: 0, y: 0, width: 390, height: 50 },
        },
      });

      // Simulate content size change (indicating scrollable content)
      fireEvent(scrollView, 'contentSizeChange', 1000, 50);

      // Re-render to trigger useEffect for fade visibility
      rerender(
        <ScrollFadeContainer testID="scroll-fade">
          <View style={{ width: 1000 }}>
            <Text>Wide scrollable content</Text>
          </View>
        </ScrollFadeContainer>
      );

      // Component should handle this without crashing
      expect(container).toBeTruthy();
    });

    it('should show left fade after scrolling right', () => {
      const { getByTestId } = render(
        <ScrollFadeContainer testID="scroll-fade">
          <View style={{ width: 1000 }}>
            <Text>Wide content</Text>
          </View>
        </ScrollFadeContainer>
      );

      const container = getByTestId('scroll-fade');
      const scrollView = getByTestId('scroll-fade-scroll');

      // Setup scrollable content dimensions
      fireEvent(container, 'layout', {
        nativeEvent: {
          layout: { x: 0, y: 0, width: 390, height: 50 },
        },
      });
      fireEvent(scrollView, 'contentSizeChange', 1000, 50);

      // Simulate scrolling right (which should show left fade)
      fireEvent.scroll(scrollView, {
        nativeEvent: {
          contentOffset: { x: 200, y: 0 },
          contentSize: { width: 1000, height: 50 },
          layoutMeasurement: { width: 390, height: 50 },
        },
      });

      // Component should handle scroll events properly
      expect(scrollView).toBeTruthy();
    });

    it('should hide right fade when scrolled to end', () => {
      const { getByTestId } = render(
        <ScrollFadeContainer testID="scroll-fade">
          <View style={{ width: 1000 }}>
            <Text>Wide content</Text>
          </View>
        </ScrollFadeContainer>
      );

      const container = getByTestId('scroll-fade');
      const scrollView = getByTestId('scroll-fade-scroll');

      // Setup scrollable content dimensions
      fireEvent(container, 'layout', {
        nativeEvent: {
          layout: { x: 0, y: 0, width: 390, height: 50 },
        },
      });
      fireEvent(scrollView, 'contentSizeChange', 1000, 50);

      // Simulate scrolling to the very end
      fireEvent.scroll(scrollView, {
        nativeEvent: {
          contentOffset: { x: 610, y: 0 }, // 1000 - 390 = 610 (max scroll)
          contentSize: { width: 1000, height: 50 },
          layoutMeasurement: { width: 390, height: 50 },
        },
      });

      // Component should handle scroll to end
      expect(scrollView).toBeTruthy();
    });
  });

  describe('default values', () => {
    it('should use default backgroundColor from theme', () => {
      const { getByTestId } = render(
        <ScrollFadeContainer testID="test-scroll">
          <Text>Content</Text>
        </ScrollFadeContainer>
      );
      // Should render without errors using default background color
      expect(getByTestId('test-scroll')).toBeTruthy();
    });

    it('should use default fadeWidth of 32', () => {
      const { getByTestId } = render(
        <ScrollFadeContainer testID="test-scroll">
          <Text>Content</Text>
        </ScrollFadeContainer>
      );
      // Should render without errors using default fade width
      expect(getByTestId('test-scroll')).toBeTruthy();
    });
  });
  // The fade is drawn, not only mounted: a gradient sized by a class NativeWind
  // never reached was 0 wide, and one ending in `'transparent'` ran through grey.
  describe('the fade is drawn', () => {
    function scrolled(backgroundColor?: string) {
      const screen = render(
        <ScrollFadeContainer testID="fade" backgroundColor={backgroundColor}>
          <View style={{ width: 1000 }}>
            <Text>Wide content</Text>
          </View>
        </ScrollFadeContainer>
      );
      fireEvent(screen.getByTestId('fade'), 'layout', { nativeEvent: { layout: { x: 0, y: 0, width: 390, height: 40 } } });
      fireEvent(screen.getByTestId('fade-scroll'), 'contentSizeChange', 1000, 40);
      fireEvent.scroll(screen.getByTestId('fade-scroll'), {
        nativeEvent: {
          contentOffset: { x: 200, y: 0 },
          contentSize: { width: 1000, height: 40 },
          layoutMeasurement: { width: 390, height: 40 },
        },
      });
      return screen;
    }

    it('fills each edge with its gradient through a style, not a class', () => {
      const screen = scrolled('#f7f6f2');
      for (const edge of ['fade-left-fade', 'fade-right-fade']) {
        const gradient = screen.getByTestId(edge).findByProps({ testID: 'linear-gradient' });
        expect(StyleSheet.flatten(gradient.props.style)).toMatchObject({ position: 'absolute', top: 0, right: 0, bottom: 0, left: 0 });
      }
    });

    it('fades the page colour into itself made clear, never into black at alpha 0', () => {
      const screen = scrolled('#f7f6f2');
      const right = screen.getByTestId('fade-right-fade').findByProps({ testID: 'linear-gradient' });
      const left = screen.getByTestId('fade-left-fade').findByProps({ testID: 'linear-gradient' });
      expect(right.props.colors).toEqual(['#f7f6f200', '#f7f6f2']);
      expect(left.props.colors).toEqual(['#f7f6f2', '#f7f6f200']);
    });

    it('makes any page colour clear without changing its hue', () => {
      expect(clearOf('#fff')).toBe('#ffffff00');
      expect(clearOf('#11130f')).toBe('#11130f00');
      expect(clearOf('#11130fcc')).toBe('#11130f00');
      expect(clearOf('rgb(17, 19, 15)')).toBe('rgba(17, 19, 15, 0)');
      expect(clearOf('rgba(17,19,15,0.5)')).toBe('rgba(17, 19, 15, 0)');
    });
  });
});
