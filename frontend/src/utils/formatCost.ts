// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Single source of truth for rendering a USD cost in the admin surfaces.
// ABOUTME: Replaces three divergent copies that disagreed on zero and on sub-cent spend.

/**
 * Format a USD cost for display.
 *
 * There were three copies of this before, and all three were wrong in a
 * different way:
 *
 * - `ActivityTab` branched on `usd < 0.01` into `toFixed(4)`, so a genuinely
 *   zero cost rendered "$0.0000" — four zeros in a money column beside
 *   two-decimal figures.
 * - `LlmConsumptionPanel` special-cased exact zero but kept `toFixed(4)` for
 *   sub-cent values, which still renders "$0.0000" for anything below $0.00005.
 * - `UserDetailDrawer` collapsed every sub-cent value to "<$0.01", which is
 *   honest but hides whether the figure is $0.0001 or $0.009.
 *
 * Sub-cent amounts are real money here, not rounding noise: `llama-3.1-8b` is
 * billed at $0.05 per million input tokens, so a day of light traffic lands
 * around $0.00008. Fixed decimal places cannot represent that without either
 * collapsing it to zero or padding every other row with dead digits, so
 * sub-cent values switch to significant digits instead.
 *
 * Note this only pays off because the server stopped rounding to two decimals
 * before serialising — while it did, every sub-cent figure arrived as exactly
 * 0.0 and this branch was unreachable.
 */
export function formatCost(usd: number): string {
  // Guard non-finite input rather than rendering "$NaN" into an operator's
  // cost dashboard.
  if (!Number.isFinite(usd)) {
    return '$0.00';
  }

  // Exactly zero is not a tiny amount — it is no spend, and it should look
  // like every other whole-cent figure in the column.
  if (usd === 0) {
    return '$0.00';
  }

  const magnitude = Math.abs(usd);
  const sign = usd < 0 ? '-' : '';

  if (magnitude < 0.01) {
    // `Number(...)` strips the trailing zeros `toPrecision` leaves behind:
    // 0.00008688 -> "0.000087" rather than "0.000087000".
    return `${sign}$${Number(magnitude.toPrecision(2))}`;
  }

  return `${sign}$${magnitude.toFixed(2)}`;
}
