// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web's binding of the shared import copy: an import's one state line, rendered through i18next
// ABOUTME: The decision of which line and which values is ui-logic's, shared with the mobile screen

import type { TFunction } from '@pierre/i18n';
import type { ArchiveImport } from '@pierre/shared-types';
import { importStateCopy } from '@pierre/ui-logic';

/** The state line of `entry`, in `locale`. */
export function importStateLine(t: TFunction, entry: ArchiveImport, locale: string): string {
  const line = importStateCopy(entry, locale);
  return t(line.key, line.values);
}
