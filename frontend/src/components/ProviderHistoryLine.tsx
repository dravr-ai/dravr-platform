// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The line a provider card shows about the athlete's past activities — copying, in, unfinished, or not here yet
// ABOUTME: Reads the card's `history` (carnet#1386); a card whose history is not applicable renders nothing

import { useTranslation } from '@pierre/i18n';
import { HISTORY_ACTION_KEYS, HISTORY_LINE_KEYS, cardHistoryState } from '@pierre/shared-constants';
import type { ExtendedProviderStatus } from '@pierre/shared-types';

/** The dot beside each state's line: none while copying or once in, a warning when unfinished or missing. */
const NEEDS_ATTENTION = new Set(['incomplete', 'needed']);

interface ProviderHistoryLineProps {
  provider: Pick<ExtendedProviderStatus, 'provider' | 'display_name' | 'connected' | 'history'>;
  /** The line's DOM id, which the card's sign-in button names as its description. */
  id: string;
}

/**
 * The card's history line. `needed` also says, once, what the sign-in does;
 * `imported` is a quiet confirmation, never a nag. The action that opens the
 * sign-in sits with the card's other actions.
 */
export default function ProviderHistoryLine({ provider, id }: ProviderHistoryLineProps) {
  const { t } = useTranslation();
  const state = cardHistoryState(provider);
  if (state === 'not_applicable') return null;
  const name = { provider: provider.display_name };
  return (
    <div
      className="mt-0.5"
      id={id}
      data-testid={`provider-history-${provider.provider}`}
      data-state={state}
    >
      <p
        className={`inline-flex items-center gap-1.5 text-xs ${state === 'imported' ? 'text-outline' : 'text-on-surface-variant'}`}
        role={state === 'importing' ? 'status' : undefined}
      >
        {NEEDS_ATTENTION.has(state) && <span aria-hidden="true" className="h-2 w-2 rounded-full bg-warning" />}
        {t(HISTORY_LINE_KEYS[state], name)}
      </p>
      {state === 'needed' && <p className="text-xs text-outline">{t(HISTORY_ACTION_KEYS.explainer, name)}</p>}
    </div>
  );
}
