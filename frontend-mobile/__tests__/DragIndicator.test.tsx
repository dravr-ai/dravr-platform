// ABOUTME: Unit tests for DragIndicator component
// ABOUTME: Tests rendering and testID propagation for the modal drag pill

import React from 'react';
import { render } from '@testing-library/react-native';
import { DragIndicator } from '../src/components/ui/DragIndicator';

describe('DragIndicator', () => {
  describe('rendering', () => {
    it('should render without crashing', () => {
      const { toJSON } = render(<DragIndicator />);
      expect(toJSON()).toBeTruthy();
    });

    it('should render a container view with the indicator pill', () => {
      const tree = render(<DragIndicator />);
      const json = tree.toJSON();

      // Root is a View container with the pill View inside
      expect(json).toBeTruthy();
      if (json && !Array.isArray(json)) {
        expect(json.type).toBe('View');
        expect(json.children).toBeTruthy();
        expect(json.children).toHaveLength(1);
      }
    });
  });

  describe('testID', () => {
    it('should pass testID to container view', () => {
      const { getByTestId } = render(<DragIndicator testID="drag-indicator" />);
      expect(getByTestId('drag-indicator')).toBeTruthy();
    });

    it('should render without testID when not provided', () => {
      const { queryByTestId } = render(<DragIndicator />);
      expect(queryByTestId('drag-indicator')).toBeNull();
    });
  });

  describe('visual structure', () => {
    it('draws the pill in the strong hairline of the scheme, not a fixed white', () => {
      const tree = render(<DragIndicator />);
      const json = tree.toJSON();

      // The pill is the child View with the inline backgroundColor style.
      // Outside a ThemeProvider the palette is the dark one, so the strong
      // hairline is the pale grey-green at the web's dark alpha.
      expect(json && !Array.isArray(json) && json.children).toBeTruthy();
      const pill = (json as { children: Array<{ props: { style: unknown } }> }).children[0];
      const styles = Array.isArray(pill.props.style) ? pill.props.style : [pill.props.style];
      const hasHairlineFill = styles.some(
        (s: Record<string, string>) => s && s.backgroundColor === 'rgba(192, 200, 195, 0.34)'
      );
      expect(hasHairlineFill).toBe(true);
    });
  });
});
