// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The dashboard tabs only an admin may open
// ABOUTME: Read by the route guard in Dashboard.tsx and by the screen-theory staleness test

/**
 * Tabs only an admin may open.
 *
 * The sidebar already offers a role-appropriate set, but the hash is
 * user-editable and `applyRoute` accepted whatever was typed — so `#users` as a
 * regular user mounted the admin pane. Kept as a literal set rather than derived
 * from the tab arrays because those are built inside the component, after the
 * route handler that needs them.
 */
export const ADMIN_ONLY_TABS: ReadonlySet<string> = new Set([
  'users', 'agents', 'agent-store', 'configuration', 'user-tools', 'prompts',
  'platform-settings', 'claim-verdicts', 'harness-config',
  'memory-worker', 'activity', 'engagement', 'connections',
  'analytics', 'billing', 'impersonation-log', 'audit',
]);
