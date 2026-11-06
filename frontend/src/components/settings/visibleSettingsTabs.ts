// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The settings sections one viewer gets, shared by the athlete menu and the operator strip
// ABOUTME: Reads the flag from each pane's declaration, so a flagged pane cannot be missed by one of the two lists

import { ADMIN_HIDDEN_PANES, settingsPane } from '@pierre/shared-constants';
import { SETTINGS_TABS, type SettingsTabDef } from './settingsTabs';

/** Operators lose the athlete-account panes, and a pane behind the API tokens flag needs it on. */
export function visibleSettingsTabs(isAdminUser: boolean, apiTokensEnabled: boolean): SettingsTabDef[] {
  return SETTINGS_TABS.filter((tab) => {
    if (isAdminUser && ADMIN_HIDDEN_PANES.has(tab.id)) return false;
    return apiTokensEnabled || settingsPane(tab.id).flag !== 'api_tokens';
  });
}
