// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A figure with a fixed number of decimals in the athlete's own notation — 42.0 in English, 42,0 in French
// ABOUTME: Every figure and count both clients print goes through here, so a card never mixes a comma with a full stop

/**
 * `value` with exactly `digits` decimals in the notation of `language` —
 * `42.00` in English, `42,00` in French — and no thousands grouping, so a
 * column of figures keeps one width per digit count.
 *
 * `toFixed` always writes a full stop whatever the athlete reads. This is
 * `Intl.NumberFormat`, the one Intl service the browser and the phone's
 * Hermes runtime both carry in full.
 */
export function formatDecimal(value: number, digits: number, language: string): string {
  return new Intl.NumberFormat(language, {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
    useGrouping: false,
  }).format(value);
}

/**
 * A whole count in the notation of `language`, grouped as that language
 * groups thousands: `12,345` in English, `12 345` in French, `12.345` in
 * German. Every usage counter and quota figure both clients print reads here
 * rather than through `toLocaleString()`, which follows the device rather
 * than the language the athlete chose.
 */
export function formatCount(value: number, language: string): string {
  return new Intl.NumberFormat(language, { maximumFractionDigits: 0 }).format(value);
}

/**
 * An amount held in the currency's minor unit (cents), as `language` writes
 * money: `$12.50` in English, `12,50 $US` in French. Invoice amounts arrive
 * from the billing provider in minor units with an ISO 4217 code, lower-case
 * as Stripe sends it.
 *
 * The minor unit is the currency's own: a cent is a hundredth of a dollar,
 * but a yen has no subdivision, so `1250` JPY is `¥1,250`. The formatter's
 * resolved fraction digits are that exponent.
 */
export function formatMinorCurrency(minorUnits: number, currency: string, language: string): string {
  const format = new Intl.NumberFormat(language, {
    style: 'currency',
    currency: currency.toUpperCase(),
  });
  // A currency format always resolves its fraction digits; the type leaves
  // them optional, and 2 is the exponent most ISO 4217 currencies carry.
  const exponent = format.resolvedOptions().maximumFractionDigits ?? 2;
  return format.format(minorUnits / 10 ** exponent);
}

/**
 * An amount held in the currency's major unit (dollars), as `language` writes
 * money, with no decimals when the amount is whole: `$20` in English, `20 $US`
 * in French, `$12.50` when it is not. A plan's included usage arrives this way,
 * where an invoice's amount arrives in minor units ({@link formatMinorCurrency}).
 */
export function formatMajorCurrency(amount: number, currency: string, language: string): string {
  const digits = Number.isInteger(amount) ? { minimumFractionDigits: 0, maximumFractionDigits: 0 } : {};
  return new Intl.NumberFormat(language, {
    style: 'currency',
    currency: currency.toUpperCase(),
    ...digits,
  }).format(amount);
}
