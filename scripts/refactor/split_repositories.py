#!/usr/bin/env python3
"""Mechanical splitter for crates/pierre-database/src/repositories.rs into per-domain files.
One-shot script; not part of build."""
import os, sys, re

SRC = 'crates/pierre-database/src/repositories.rs'
OUT_DIR = 'crates/pierre-database/src/repositories'

LICENSE_HDR = """// ABOUTME: Repository trait definitions for the {desc} domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
"""

# Items: (start_line, end_line, name) — 1-based inclusive ranges already validated.
ITEMS = {
    'SyncCursorRow': (80,107),
    'ConnectedUserRow': (109,116),
    'UserRepository': (122,211),
    'OAuthTokenRepository': (213,290),
    'ApiKeyRepository': (292,320),
    'UsageRepository': (322,359),
    'A2ARepository': (361,444),
    'ProfileRepository': (446,468),
    'InsightRepository': (470,482),
    'AdminRepository': (484,534),
    'TenantRepository': (536,576),
    'OAuth2ServerRepository': (578,639),
    'SecurityRepository': (641,707),
    'NotificationRepository': (709,730),
    'FitnessConfigRepository': (732,778),
    'ChatRepository': (780,888),
    'UpsertUserFactParams': (894,921),
    'InsertCompactionBlockParams': (923,939),
    'InsertCoachNoteParams': (941,957),
    'InsertCoachFollowupParams': (959,973),
    'HarnessMemoryRepository': (975,1166),
    'InsertClaimVerdictParams': (1172,1200),
    'VerdictStatusBreakdown': (1202,1215),
    'VerdictDailyBucket': (1217,1224),
    'VerdictCalibrationStats': (1226,1242),
    'ClaimVerdictRepository': (1244,1278),
    'UserMcpTokenRepository': (1280,1299),
    'ImpersonationRepository': (1301,1322),
    'LlmCredentialRepository': (1324,1359),
    'SubscriptionsRepository': (1361,1410),
    'ProviderConnectionRepository': (1412,1439),
    'PasswordResetRepository': (1441,1471),
    'WeatherCacheEntry': (1473,1497),
    'WeatherCacheRepository': (1499,1518),
    'OAuthClientStateRepository': (1520,1532),
    'ToolSelectionRepository': (1534,1574),
    'LlmUsageRepository': (1576,1654),
    'UsageCounterRepository': (1656,1683),
    'RecipeRepository': (1689,1744),
    'CoachesRepository': (1746,1967),
    'MobilityRepository': (1969,2010),
    'SocialRepository': (2012,2161),
    'UpsertChannelConfigParams': (2163,2187),
    'CreateSessionParams': (2189,2205),
    'InsertMessageParams': (2207,2231),
    'CreateLinkStateParams': (2233,2253),
    'CreateChannelLinkParams': (2255,2269),
    'MessagingRepository': (2271,2507),
    'SeedTable_enum': (2513,2546),
    'SeedTable_impl': (2548,2570),
    'SeederRepository': (2572,2765),
    'StoreListingsRepository': (2769,2872),
    'CoachingGroupRepository': (2878,3000),
    'DataSourceRepository': (3006,3032),
    'SleepRepository': (3034,3086),
    'RecoveryRepository': (3088,3127),
    'HealthSnapshotRepository': (3129,3168),
    'TimeSeriesPointRepository': (3174,3208),
    'SyncCursorRepository': (3214,3234),
    'UserPhysiologicalProfileRepository': (3240,3270),
    'DossierRepository': (3272,3288),
    'PrescribedWorkoutRepository': (3290,3308),
    'WorkoutTemplateRepository': (3310,3342),
    'RouteSummaryRepository': (3344,3373),
    'TrainingHistoryRepository': (3375,3416),
    'RosterRepository': (3418,3472),
    'DatabaseProvider': (3478,3492),
}

DOMAINS = [
    ('users', 'user/profile/impersonation/physiology', [
        'UserRepository','ProfileRepository','PasswordResetRepository',
        'ImpersonationRepository','UserPhysiologicalProfileRepository','DossierRepository',
    ]),
    ('oauth', 'OAuth tokens, OAuth2 server, client state, provider connections', [
        'OAuthTokenRepository','OAuth2ServerRepository','OAuthClientStateRepository','ProviderConnectionRepository',
    ]),
    ('api_keys', 'API keys and user MCP tokens', [
        'ApiKeyRepository','UserMcpTokenRepository',
    ]),
    ('usage', 'API/LLM/usage-counter accounting + LLM credentials', [
        'UsageRepository','UsageCounterRepository','LlmUsageRepository','LlmCredentialRepository',
    ]),
    ('a2a', 'agent-to-agent (A2A) protocol persistence', [
        'A2ARepository',
    ]),
    ('insights', 'persisted insights', [
        'InsightRepository',
    ]),
    ('admin', 'admin tokens and admin overrides', [
        'AdminRepository',
    ]),
    ('tenants', 'tenants and subscriptions', [
        'TenantRepository','SubscriptionsRepository',
    ]),
    ('security', 'security/audit/key-version persistence', [
        'SecurityRepository',
    ]),
    ('notifications', 'OAuth + system notification persistence', [
        'NotificationRepository',
    ]),
    ('fitness_config', 'fitness configuration persistence', [
        'FitnessConfigRepository',
    ]),
    ('chat', 'chat conversation persistence', [
        'ChatRepository',
    ]),
    ('harness_memory', 'coaching harness memory (Tier 0 foundations)', [
        'UpsertUserFactParams','InsertCompactionBlockParams','InsertCoachNoteParams','InsertCoachFollowupParams','HarnessMemoryRepository',
    ]),
    ('claim_verdicts', 'claim verdict (bullshit detector) persistence', [
        'InsertClaimVerdictParams','VerdictStatusBreakdown','VerdictDailyBucket','VerdictCalibrationStats','ClaimVerdictRepository',
    ]),
    ('weather', 'weather cache persistence', [
        'WeatherCacheEntry','WeatherCacheRepository',
    ]),
    ('tool_selection', 'tool selection telemetry persistence', [
        'ToolSelectionRepository',
    ]),
    ('recipes', 'recipe persistence', [
        'RecipeRepository',
    ]),
    ('coaches', 'coaches catalogue, coaching groups, store listings', [
        'CoachesRepository','CoachingGroupRepository','StoreListingsRepository',
    ]),
    ('mobility', 'mobility (yoga / stretching) persistence', [
        'MobilityRepository',
    ]),
    ('social', 'social graph persistence', [
        'SocialRepository',
    ]),
    ('messaging', 'inbound/outbound messaging channel persistence', [
        'UpsertChannelConfigParams','CreateSessionParams','InsertMessageParams','CreateLinkStateParams','CreateChannelLinkParams','MessagingRepository',
    ]),
    ('seeder', 'seed-only repository operations', [
        'SeedTable_enum','SeedTable_impl','SeederRepository',
    ]),
    ('data_source', 'data source registration persistence', [
        'DataSourceRepository',
    ]),
    ('health', 'health/sleep/recovery/time-series persistence + sync cursors', [
        'SyncCursorRow','ConnectedUserRow','SleepRepository','RecoveryRepository','HealthSnapshotRepository','TimeSeriesPointRepository','SyncCursorRepository',
    ]),
    ('workouts', 'prescribed workouts, templates, routes, training history', [
        'PrescribedWorkoutRepository','WorkoutTemplateRepository','RouteSummaryRepository','TrainingHistoryRepository',
    ]),
    ('roster', 'coach-athlete roster persistence', [
        'RosterRepository',
    ]),
]

def main():
    src_lines = open(SRC).read().splitlines(keepends=False)
    # Imports: lines 1..74 inclusive (1-based) -> indices 0..73.
    # Drop the SPDX header (lines 1-5) and start with `use` block (line 7..74).
    imports_block = '\n'.join(src_lines[6:74])  # lines 7..74

    # Sanity check: every item used exactly once across domains.
    used = []
    for _, _, items in DOMAINS:
        used.extend(items)
    assert len(used) == len(set(used)), 'dup item across domains'
    # DatabaseProvider stays in mod.rs; everything else must be in some domain.
    expected = set(ITEMS.keys()) - {'DatabaseProvider'}
    assert set(used) == expected, f'mismatch: missing={expected-set(used)} extra={set(used)-expected}'

    os.makedirs(OUT_DIR, exist_ok=True)
    for mod_name, desc, item_names in DOMAINS:
        out = []
        out.append(LICENSE_HDR.format(desc=desc).rstrip())
        out.append('')
        out.append(imports_block)
        out.append('')
        for name in item_names:
            s, e = ITEMS[name]
            chunk = src_lines[s-1:e]
            out.append('\n'.join(chunk))
            out.append('')
        with open(os.path.join(OUT_DIR, f'{mod_name}.rs'), 'w') as f:
            f.write('\n'.join(out).rstrip() + '\n')

    # mod.rs: pub mod X for each domain + re-exports + DatabaseProvider trait kept here
    s, e = ITEMS['DatabaseProvider']
    db_provider_chunk = '\n'.join(src_lines[s-1:e])
    mod_lines = []
    mod_lines.append('// ABOUTME: Repository module — re-exports the per-domain repository traits and types')
    mod_lines.append('// ABOUTME: Per-domain files were split out of the original 3500-line repositories.rs (Finding B)')
    mod_lines.append('//')
    mod_lines.append('// SPDX-License-Identifier: MIT OR Apache-2.0')
    mod_lines.append('// Copyright (c) 2026 dravr.ai')
    mod_lines.append('')
    for mod_name, _, _ in DOMAINS:
        mod_lines.append(f'pub mod {mod_name};')
    mod_lines.append('')
    for mod_name, _, _ in DOMAINS:
        mod_lines.append(f'pub use {mod_name}::*;')
    mod_lines.append('')
    mod_lines.append('use async_trait::async_trait;')
    mod_lines.append('use pierre_core::errors::AppResult;')
    mod_lines.append('')
    mod_lines.append(db_provider_chunk)
    with open(os.path.join(OUT_DIR, 'mod.rs'), 'w') as f:
        f.write('\n'.join(mod_lines).rstrip() + '\n')

    # Remove original
    os.remove(SRC)
    print(f'OK: wrote {len(DOMAINS)} domain files + mod.rs; removed {SRC}')

if __name__ == '__main__':
    main()
