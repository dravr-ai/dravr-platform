// ABOUTME: Unit tests for date and text formatting utilities
// ABOUTME: Tests formatDuration, formatDistance, formatPace, truncateText

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
  formatDuration,
  formatDistance,
  formatPace,
  truncateText,
} from '../src/formatting';

describe('formatDuration', () => {
  it('formats hours, minutes, and seconds', () => {
    expect(formatDuration(3665)).toBe('1h 1m 5s');
  });

  it('formats only minutes and seconds', () => {
    expect(formatDuration(125)).toBe('2m 5s');
  });

  it('formats only seconds', () => {
    expect(formatDuration(45)).toBe('45s');
  });

  it('formats zero as 0s', () => {
    expect(formatDuration(0)).toBe('0s');
  });

  it('handles exact hours', () => {
    expect(formatDuration(7200)).toBe('2h');
  });

  it('handles exact minutes', () => {
    expect(formatDuration(300)).toBe('5m');
  });

  it('handles large durations', () => {
    expect(formatDuration(36000)).toBe('10h');
  });
});

describe('formatDistance', () => {
  describe('metric', () => {
    it('formats meters under 1000 as m', () => {
      expect(formatDistance(500)).toBe('500 m');
    });

    it('formats meters >= 1000 as km', () => {
      expect(formatDistance(5000)).toBe('5.00 km');
    });

    it('formats partial kilometers', () => {
      expect(formatDistance(1500)).toBe('1.50 km');
    });

    it('handles zero', () => {
      expect(formatDistance(0)).toBe('0 m');
    });
  });

  describe('imperial', () => {
    it('formats as miles for >= 1 mile', () => {
      expect(formatDistance(5000, 'imperial')).toBe('3.11 mi');
    });

    it('formats as feet for < 1 mile', () => {
      expect(formatDistance(100, 'imperial')).toBe('328 ft');
    });
  });
});

describe('formatPace', () => {
  it('formats pace in metric', () => {
    expect(formatPace(300)).toBe('5:00 /km');
  });

  it('formats pace in imperial', () => {
    expect(formatPace(480, 'imperial')).toBe('8:00 /mi');
  });

  it('pads seconds with zero', () => {
    expect(formatPace(305)).toBe('5:05 /km');
  });

  it('handles sub-minute pace', () => {
    expect(formatPace(45)).toBe('0:45 /km');
  });
});

describe('truncateText', () => {
  it('returns text unchanged if within limit', () => {
    expect(truncateText('hello', 10)).toBe('hello');
  });

  it('truncates with ellipsis when over limit', () => {
    expect(truncateText('hello world this is long', 10)).toBe('hello w...');
  });

  it('returns text unchanged if exactly at limit', () => {
    expect(truncateText('12345', 5)).toBe('12345');
  });

  it('handles empty string', () => {
    expect(truncateText('', 10)).toBe('');
  });
});
