// ABOUTME: Worked example shown on the connect gate — what coaching looks like, before handing over credentials
// ABOUTME: Explicitly labelled as an example throughout; never dressed up as the viewer's own data

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useTranslation } from '@pierre/i18n';

/**
 * A short illustrative exchange, shown collapsed on the connect gate.
 *
 * The gate asks for a third-party fitness password before the user has seen a
 * single thing the product does. That ordering is not negotiable — the coach
 * genuinely has nothing to reason about without activity data — so the fix is to
 * show what they are being asked to unlock rather than to move the gate.
 *
 * Every line is labelled as an example and uses a named fictional athlete. It
 * must never be mistakable for the viewer's own data: someone who thinks we
 * already have their numbers before they connected anything has been misled, and
 * that is a worse outcome than an unpersuasive gate.
 */
export default function ConnectPreview() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);

  return (
    <div className="mt-6">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="w-full text-sm font-medium text-on-surface-variant hover:text-on-surface underline-offset-2 hover:underline transition-colors"
      >
        {open ? t('shell.previewExampleHide') : t('shell.previewSeeExample')}
      </button>

      {open && (
        <div className="mt-4 rounded-xl border border-outline-variant bg-surface-container-low p-4">
          <p className="text-xs uppercase tracking-wide text-on-surface-variant font-label">
            {t('shell.previewExampleBadge')}
          </p>
          <p className="mt-1 text-xs text-on-surface-variant">
            {t('shell.previewExampleCaption')}
          </p>

          <div className="mt-4 flex flex-col gap-3">
            <div className="self-end max-w-[85%] rounded-2xl rounded-br-sm border border-primary bg-primary/10 px-3.5 py-2">
              <p className="text-sm text-on-surface">
                {t('shell.previewExampleMessage')}
              </p>
            </div>

            <div className="self-start max-w-[90%] rounded-2xl rounded-bl-sm border border-outline-variant bg-surface-container px-3.5 py-2">
              <p className="text-sm text-on-surface">
                Your load is up 24% on last week and you slept under six hours twice — heavy legs
                are the expected result, not a warning sign. Keep Saturday, drop it to easy pace
                and cut the last 5k. That protects the long-run habit without adding to a week
                that is already the biggest of your block.
              </p>
            </div>
          </div>

          <p className="mt-4 text-xs text-on-surface-variant">
            {t('shell.previewSpecificsHint')}
          </p>
        </div>
      )}
    </div>
  );
}
