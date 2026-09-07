// ABOUTME: Unit tests for the mobile scale tokens — spacing, type scale, radius
// ABOUTME: Colour is not testable from here: it lives in useThemeColors(), which needs a scheme to resolve

import { spacing, fontSize, borderRadius } from '../src/constants/theme';

describe('Theme Constants', () => {
  describe('spacing', () => {
    it('should have spacing values', () => {
      expect(spacing.xs).toBeDefined();
      expect(spacing.sm).toBeDefined();
      expect(spacing.md).toBeDefined();
      expect(spacing.lg).toBeDefined();
      expect(spacing.xl).toBeDefined();
    });

    it('should have increasing spacing values', () => {
      expect(spacing.sm).toBeGreaterThan(spacing.xs);
      expect(spacing.md).toBeGreaterThan(spacing.sm);
      expect(spacing.lg).toBeGreaterThan(spacing.md);
      expect(spacing.xl).toBeGreaterThan(spacing.lg);
    });
  });

  describe('fontSize', () => {
    it('should have font size values', () => {
      expect(fontSize.xs).toBeDefined();
      expect(fontSize.sm).toBeDefined();
      expect(fontSize.md).toBeDefined();
      expect(fontSize.lg).toBeDefined();
      expect(fontSize.xl).toBeDefined();
    });

    it('should have increasing font sizes', () => {
      expect(fontSize.sm).toBeGreaterThan(fontSize.xs);
      expect(fontSize.md).toBeGreaterThan(fontSize.sm);
      expect(fontSize.lg).toBeGreaterThan(fontSize.md);
      expect(fontSize.xl).toBeGreaterThan(fontSize.lg);
    });
  });

  describe('borderRadius', () => {
    it('should have increasing border radii, with a pill at the top', () => {
      expect(borderRadius.md).toBeGreaterThan(borderRadius.sm);
      expect(borderRadius.lg).toBeGreaterThan(borderRadius.md);
      expect(borderRadius.xl).toBeGreaterThan(borderRadius.lg);
      // `full` is a pill, not the next step up — it has to exceed any height
      // a chip or avatar can take.
      expect(borderRadius.full).toBeGreaterThan(1000);
    });
  });
});
