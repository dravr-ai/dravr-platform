// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Page controls for the fuel review queue and the live catalogue — which rows of the total are shown, and the previous and next pages
// ABOUTME: The server answers each page with its total, so nothing past the first page is ever out of reach

import { Button } from '../ui';

export default function Pager({
  offset,
  pageSize,
  shown,
  total,
  noun,
  onOffset,
  testId,
}: {
  offset: number;
  pageSize: number;
  /** Rows on this page. */
  shown: number;
  total: number;
  /** What a row is, plural ("proposals"). */
  noun: string;
  onOffset: (offset: number) => void;
  testId: string;
}) {
  if (total <= pageSize && offset === 0) return null;
  const first = shown === 0 ? offset : offset + 1;
  return (
    <div className="flex items-center justify-between gap-3 pt-3" data-testid={testId}>
      <p className="text-xs text-on-surface-variant" aria-live="polite">
        {first}–{offset + shown} of {total} {noun}
      </p>
      <div className="flex gap-2">
        <Button
          type="button"
          variant="secondary"
          size="sm"
          disabled={offset === 0}
          onClick={() => onOffset(Math.max(0, offset - pageSize))}
        >
          Previous
        </Button>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          disabled={offset + shown >= total}
          onClick={() => onOffset(offset + pageSize)}
        >
          Next
        </Button>
      </div>
    </div>
  );
}
