// ABOUTME: Date and text formatting utilities shared across web and mobile
// ABOUTME: Pure functions with no platform-specific dependencies

/**
 * The translator `formatDuration` takes — i18next's `t` fits it. The shared
 * package holds no catalogue of its own and no hook.
 */
export type DurationTranslate = (key: string, options: { value: number }) => string;

/**
 * The catalogue keys of the three units a duration is written in, each
 * `{{value}}` plus the unit as the language abbreviates it: `1h`/`30m`/`10s`
 * in English, `1 h`/`30 min`/`10 s` in French, `1 Std.`/`30 Min.`/`10 Sek.`
 * in German. The words come from the catalogue rather than from
 * `Intl.NumberFormat`'s unit style, which the phone's JavaScript engine
 * renders through a different formatter on each platform, or not at all.
 */
export const DURATION_UNIT_KEYS = {
  hours: 'common.duration.hours',
  minutes: 'common.duration.minutes',
  seconds: 'common.duration.seconds',
} as const;

const SECONDS_PER_HOUR = 3600;
const SECONDS_PER_MINUTE = 60;

/**
 * A duration in whole seconds, in the athlete's language: `1h 1m 5s` in
 * English, `1 h 1 min 5 s` in French. A unit that is zero is left out, and a
 * zero duration reads as zero seconds.
 */
export function formatDuration(t: DurationTranslate, seconds: number): string {
  const whole = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(whole / SECONDS_PER_HOUR);
  const minutes = Math.floor((whole % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE);
  const secs = whole % SECONDS_PER_MINUTE;

  const parts: string[] = [];
  if (hours > 0) parts.push(t(DURATION_UNIT_KEYS.hours, { value: hours }));
  if (minutes > 0) parts.push(t(DURATION_UNIT_KEYS.minutes, { value: minutes }));
  if (secs > 0 || parts.length === 0) parts.push(t(DURATION_UNIT_KEYS.seconds, { value: secs }));

  return parts.join(' ');
}

/**
 * Truncate text to a maximum length with ellipsis
 */
export function truncateText(text: string, maxLength: number): string {
  if (text.length <= maxLength) return text;
  return text.slice(0, maxLength - 3) + '...';
}
