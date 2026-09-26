// ABOUTME: Unit tests for the mobile scale tokens — spacing, type scale, radius
// ABOUTME: Colour is not testable from here: it lives in useThemeColors(), which needs a scheme to resolve

import { spacing, typeScale, borderRadius } from '../src/constants/theme';

const tailwind = require('../tailwind.config.js') as {
  theme: { extend: { fontSize: Record<string, [string, { lineHeight: string }]> } };
};

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

  describe('typeScale', () => {
    // The class path and the inline path are one ladder: a style that cannot
    // take a class reads the same step a `text-*` class renders.
    it('is the text-* class ladder, step for step, in points', () => {
      const classes = tailwind.theme.extend.fontSize;
      expect(Object.keys(typeScale)).toEqual(Object.keys(classes));
      for (const [step, { fontSize, lineHeight }] of Object.entries(typeScale)) {
        expect(classes[step]).toEqual([`${fontSize}px`, { lineHeight: `${lineHeight}px` }]);
      }
    });

    it('reads 16 / 22 at the reading step, 13 / 18 at the interface step and 17 / 22 at the inline title', () => {
      expect(typeScale.base).toEqual({ fontSize: 16, lineHeight: 22 });
      expect(typeScale.sm).toEqual({ fontSize: 13, lineHeight: 18 });
      expect(typeScale.lg).toEqual({ fontSize: 17, lineHeight: 22 });
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
