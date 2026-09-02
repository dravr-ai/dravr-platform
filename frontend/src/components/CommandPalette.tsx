// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Slash-command autocomplete rendered above the web composer
// ABOUTME: Draws the server's per-caller catalogue; holds no command list of its own

import { clsx } from 'clsx';
import type { CommandEntry } from '@pierre/shared-types';
import { commandDomainLabelKey } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';

export interface CommandPaletteProps {
  /** The commands to offer, already filtered and ordered by the server. */
  matches: CommandEntry[];
  /** Index of the highlighted row, driven by the composer's arrow keys. */
  highlightedIndex: number;
  /** Fill the composer with this command. */
  onSelect: (entry: CommandEntry) => void;
}

/**
 * The `/` autocomplete over the composer.
 *
 * Every row is a command the server said this caller may run — the listing is
 * resolved per caller by the same availability predicates `/help` asks, so an
 * athlete in no group is never shown `/group invite`. Renders nothing when
 * there is nothing to offer, which is what closes it.
 */
export default function CommandPalette({
  matches,
  highlightedIndex,
  onSelect,
}: CommandPaletteProps) {
  const { t } = useTranslation();
  const domainLabel = (domain: string) => {
    const key = commandDomainLabelKey(domain);
    return key === null ? domain : t(key);
  };
  if (matches.length === 0) return null;

  return (
    <div
      role="listbox"
      aria-label={t('shell.commandPaletteSlashCommands')}
      data-testid="command-palette"
      className="mb-2 max-h-64 overflow-y-auto rounded-xl border ghost-border bg-surface-container-low shadow-ambient"
    >
      {matches.map((entry, index) => (
        <button
          key={entry.name}
          type="button"
          role="option"
          aria-selected={index === highlightedIndex}
          data-testid={`command-palette-option-${entry.name}`}
          // Keep the composer focused: a blur here would close the palette
          // before the click lands and the selection would never happen.
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => onSelect(entry)}
          className={clsx(
            'w-full text-left px-4 py-2.5 flex items-baseline gap-3 transition-colors',
            index === highlightedIndex ? 'bg-surface-container-high' : 'hover:bg-surface-container',
          )}
        >
          <span className="font-mono text-sm text-on-surface whitespace-nowrap">
            {entry.command}
            {entry.args !== null && (
              <span className="text-outline"> {entry.args}</span>
            )}
          </span>
          <span className="text-xs text-on-surface-variant truncate">{entry.description}</span>
          <span className="ml-auto text-[10px] font-label uppercase tracking-wide text-outline whitespace-nowrap">
            {domainLabel(entry.domain)}
          </span>
        </button>
      ))}
    </div>
  );
}
