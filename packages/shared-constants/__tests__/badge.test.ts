// ABOUTME: The badge cap every tab, bell and conversation row prints, pinned at its boundary
// ABOUTME: Red if the cap moves on one badge, or a count at or under it stops reading as the figure

import { describe, expect, it } from 'vitest';
import { badgeLabel } from '../src/badge';

describe('badgeLabel', () => {
  it('prints the count as a figure up to 99', () => {
    expect(badgeLabel(1)).toBe('1');
    expect(badgeLabel(10)).toBe('10');
    expect(badgeLabel(99)).toBe('99');
  });

  it('stops growing at three characters', () => {
    expect(badgeLabel(100)).toBe('99+');
    expect(badgeLabel(4321)).toBe('99+');
  });
});
