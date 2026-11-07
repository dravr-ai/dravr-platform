// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A provider card's "Have an export? Import it" link, opening the import window on that provider's export
// ABOUTME: Shown only for the export source the server put on the card and this build carries the copy of

import { useTranslation } from '@pierre/i18n';
import type { ExtendedProviderStatus } from '@pierre/shared-types';
import { cardExportSource } from '@pierre/ui-logic';
import { importWindowHash, openImportWindow } from './openImportWindow';

interface ProviderExportLinkProps {
  provider: Pick<ExtendedProviderStatus, 'provider' | 'display_name' | 'history'>;
}

export default function ProviderExportLink({ provider }: ProviderExportLinkProps) {
  const { t } = useTranslation();
  const source = cardExportSource(provider);
  if (!source) return null;
  const label = t('providers.import.cardLink');
  return (
    <button
      type="button"
      onClick={(event) => {
        // The link may sit inside a row that is itself the connect button.
        event.stopPropagation();
        openImportWindow(importWindowHash(source));
      }}
      // Several cards carry the same words: the name keeps them first and
      // adds which provider's export it opens (WCAG 2.5.3, label in name).
      aria-label={`${label} (${provider.display_name})`}
      data-testid={`provider-export-${provider.provider}`}
      className="mt-0.5 text-left text-xs font-medium text-primary hover:text-primary-hover focus:outline-none focus-visible:ring-2 focus-visible:ring-primary/50 rounded"
    >
      {label} <span aria-hidden="true">›</span>
    </button>
  );
}
