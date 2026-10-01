// ABOUTME: Unit tests for date and text formatting utilities
// ABOUTME: Tests formatDuration in all five catalogue languages, formatDistance, formatPace, truncateText

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import en from '../../i18n/src/locales/en/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import de from '../../i18n/src/locales/de/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import {
  type DurationTranslate,
  formatDuration,
  formatDistance,
  formatPace,
  truncateText,
} from '../src/formatting';

// The athlete's own catalogue, read the way i18next reads a `{{value}}`
// placeholder: each language writes its units its own way, so a duration
// built from fixed English letters fails every assertion but the English one.
const CATALOGUES = { en, fr, de, es, pt } as const;
type Language = keyof typeof CATALOGUES;

function translator(language: Language): DurationTranslate {
  return (key, { value }) => {
    const template = key.split('.').reduce<unknown>(
      (node, part) => (node as Record<string, unknown>)[part],
      CATALOGUES[language],
    );
    if (typeof template !== 'string') throw new Error(`${language} has no ${key}`);
    return template.replace('{{value}}', String(value));
  };
}

describe('formatDuration', () => {
  const t = translator('en');

  it('formats hours, minutes, and seconds', () => {
    expect(formatDuration(t, 3665)).toBe('1h 1m 5s');
  });

  it('formats only minutes and seconds', () => {
    expect(formatDuration(t, 125)).toBe('2m 5s');
  });

  it('formats only seconds', () => {
    expect(formatDuration(t, 45)).toBe('45s');
  });

  it('formats zero as 0s', () => {
    expect(formatDuration(t, 0)).toBe('0s');
  });

  it('handles exact hours', () => {
    expect(formatDuration(t, 7200)).toBe('2h');
  });

  it('handles exact minutes', () => {
    expect(formatDuration(t, 300)).toBe('5m');
  });

  it('handles large durations', () => {
    expect(formatDuration(t, 36000)).toBe('10h');
  });

  it('drops a fraction of a second rather than printing it', () => {
    expect(formatDuration(t, 2745.9)).toBe('45m 45s');
  });

  it.each<[Language, string, string, string]>([
    ['en', '45m 45s', '1h 30m 10s', '2h'],
    ['fr', '45 min 45 s', '1 h 30 min 10 s', '2 h'],
    ['de', '45 Min. 45 Sek.', '1 Std. 30 Min. 10 Sek.', '2 Std.'],
    ['es', '45 min 45 s', '1 h 30 min 10 s', '2 h'],
    ['pt', '45 min 45 s', '1 h 30 min 10 s', '2 h'],
  ])('writes a duration the way %s abbreviates its units', (language, underAnHour, overAnHour, wholeHours) => {
    const localized = translator(language);
    expect(formatDuration(localized, 2745)).toBe(underAnHour);
    expect(formatDuration(localized, 5410)).toBe(overAnHour);
    expect(formatDuration(localized, 7200)).toBe(wholeHours);
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
