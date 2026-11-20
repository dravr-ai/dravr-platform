// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The settings hash subview — `settings/<pane>`, or `settings/privacy/data-access/<id>` on one request — parsed back
// ABOUTME: A data access reminder opens its request in the privacy pane, where its letter and due date are

import { WEB_DATA_ACCESS_SUBVIEW } from '@pierre/shared-constants';

/** The pane a settings subview names, and the data access request it opens there. */
export interface SettingsSubview {
  /** The raw pane segment, for the Dashboard to check against the panes it serves. */
  pane: string;
  /** The data access request to open in the privacy pane, or `null`. */
  dataAccessRequest: string | null;
}

/**
 * Split a settings subview — `<pane>`, or `privacy/data-access/<id>` — into
 * its pane and the data access request it opens. Any other trailing shape, or
 * a malformed escape in a hand-typed hash, keeps the pane and opens no request.
 */
export function parseSettingsSubview(sub: string): SettingsSubview {
  const [pane = '', ...rest] = sub.split('/');
  if (pane !== 'privacy' || rest.length !== 2 || rest[0] !== WEB_DATA_ACCESS_SUBVIEW || rest[1] === '') {
    return { pane, dataAccessRequest: null };
  }
  try {
    return { pane, dataAccessRequest: decodeURIComponent(rest[1]) };
  } catch (error) {
    if (error instanceof URIError) return { pane, dataAccessRequest: null };
    throw error;
  }
}
