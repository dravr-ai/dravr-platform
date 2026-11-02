// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web's binding of the shared import copy: an import's one state line, rendered through i18next
// ABOUTME: The decision of which line and which values is ui-logic's, shared with the mobile screen

import type { TFunction } from '@pierre/i18n';
import type { ArchiveImport } from '@pierre/shared-types';
import { importStateCopy, type CopyLine, type ImportSourceCopy } from '@pierre/ui-logic';

/** One shared copy line, rendered. */
export function copyText(t: TFunction, line: CopyLine): string {
  return t(line.key, line.values);
}

/** The state line of `entry`, an import of `source`, in `locale`. */
export function importStateLine(t: TFunction, entry: ArchiveImport, source: ImportSourceCopy, locale: string): string {
  return copyText(t, importStateCopy(entry, source, locale));
}
