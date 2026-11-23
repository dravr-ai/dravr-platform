// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The spans a device health sync re-reads whole — sleep days (local noon to the next local noon) and local calendar dates
// ABOUTME: Pure arithmetic over instants and UTC offsets, so a night crossing midnight or noon lands in the windows it touches

const HOUR_MS = 3_600_000;
export const DAY_MS = 24 * HOUR_MS;

/** A sample's span and the offset of the clock it was recorded on. */
export interface LocalSpan {
  start: string;
  end: string;
  utcOffsetSeconds: number;
}

/** An instant span, in epoch milliseconds. */
export interface Window {
  from: number;
  to: number;
}

/** The local day number (days since 1970-01-01, local) an epoch-ms instant falls on, shifted back by `shiftMs`. */
function localDay(at: number, utcOffsetSeconds: number, shiftMs: number): number {
  return Math.floor((at + utcOffsetSeconds * 1000 - shiftMs) / DAY_MS);
}

/**
 * The sleep-day windows a span touches: each local noon to the next local
 * noon, under the span's own offset. A night from 23:00 to 07:00 is one
 * sleep day; a nap from 11:00 to 13:00 touches two.
 */
export function sleepDayWindows(span: LocalSpan): Window[] {
  const start = Date.parse(span.start);
  const end = Date.parse(span.end);
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) return [];
  const offsetMs = span.utcOffsetSeconds * 1000;
  const first = localDay(start, span.utcOffsetSeconds, 12 * HOUR_MS);
  const last = localDay(end, span.utcOffsetSeconds, 12 * HOUR_MS);
  const windows: Window[] = [];
  for (let day = first; day <= last; day += 1) {
    const from = day * DAY_MS + 12 * HOUR_MS - offsetMs;
    windows.push({ from, to: from + DAY_MS });
  }
  return windows;
}

/**
 * Windows merged where they overlap. Windows that only touch end to end stay
 * apart: two nights a noon apart are two requests' worth, never one span
 * longer than the server reads.
 */
export function mergeOverlapping(windows: Window[]): Window[] {
  const sorted = [...windows].sort((a, b) => a.from - b.from);
  const merged: Window[] = [];
  for (const window of sorted) {
    const last = merged[merged.length - 1];
    if (last && window.from < last.to) {
      last.to = Math.max(last.to, window.to);
    } else {
      merged.push({ ...window });
    }
  }
  return merged;
}

function civil(day: number): string {
  return new Date(day * DAY_MS).toISOString().slice(0, 10);
}

/** The local `YYYY-MM-DD` dates a span covers, under its own offset. */
export function localDates(span: LocalSpan): string[] {
  const start = Date.parse(span.start);
  const end = Date.parse(span.end);
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) return [];
  const dates: string[] = [];
  for (let day = localDay(start, span.utcOffsetSeconds, 0); day <= localDay(end, span.utcOffsetSeconds, 0); day += 1) {
    dates.push(civil(day));
  }
  return dates;
}

/** Every local date from `since` to `now`, under the phone's offset at `now`. */
export function datesBetween(since: string, now: Date): string[] {
  const offsetSeconds = -now.getTimezoneOffset() * 60;
  return localDates({ start: since, end: now.toISOString(), utcOffsetSeconds: offsetSeconds });
}

export function isoOf(at: number): string {
  return new Date(at).toISOString();
}
