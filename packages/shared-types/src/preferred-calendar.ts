// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The calendar Dravr's plan and workout pushes go to — the wire shape of /api/me/preferred-calendar (carnet#1393)
// ABOUTME: With its parser, so a malformed body is an error rather than a setting that silently shows nothing chosen

/** One calendar, as a push names it (`provider`) and as the athlete reads it (`name`). */
export interface CalendarOption {
  /** The provider backend that writes it, e.g. `intervals_icu`, `garmin`, `wahoo`. */
  provider: string;
  /** Its display name, e.g. "Intervals.icu". */
  name: string;
}

/** `GET`, `PUT` and `DELETE /api/me/preferred-calendar`. */
export interface PreferredCalendar {
  /** The calendar the athlete chose, or null while they chose none and a push with several asks. */
  chosen: CalendarOption | null;
  /** Whether `chosen` is among `calendars`; false when it is no longer connected and pushes pass it over. */
  chosen_connected: boolean;
  /** The athlete's own connected calendars, in the order a push offers them. */
  calendars: CalendarOption[];
}

function parseCalendarOption(value: unknown): CalendarOption | null {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return null;
  const { provider, name } = value as Record<string, unknown>;
  if (typeof provider !== 'string' || provider === '' || typeof name !== 'string') return null;
  return { provider, name };
}

/** Read a `/api/me/preferred-calendar` body, or null when it is not one. */
export function parsePreferredCalendar(body: unknown): PreferredCalendar | null {
  if (typeof body !== 'object' || body === null || Array.isArray(body)) return null;
  const { chosen, chosen_connected: chosenConnected, calendars } = body as Record<string, unknown>;
  if (typeof chosenConnected !== 'boolean' || !Array.isArray(calendars)) return null;
  const options = calendars.map(parseCalendarOption);
  if (options.some((option) => option === null)) return null;
  let chosenOption: CalendarOption | null = null;
  if (chosen !== null) {
    chosenOption = parseCalendarOption(chosen);
    if (chosenOption === null) return null;
  }
  return {
    chosen: chosenOption,
    chosen_connected: chosenConnected,
    calendars: options as CalendarOption[],
  };
}
