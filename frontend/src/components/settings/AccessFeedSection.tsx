// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: "Who read your data" on the web privacy pane — outside apps' calls and agent turns, API keys', the coach's and group members' reads per day
// ABOUTME: A Connected app carries a Revoke that revokes every grant it holds; an API key or token points to its settings (carnet#1543)

import { useMemo, useState } from 'react';
import type { AccessFeedAppRead, AccessFeedCoachDay, AccessFeedPeerDay } from '@pierre/shared-types';
import { countKey, formatDate, formatDateTime } from '@pierre/chat-utils';
import {
  ACCESS_FEED_COACH_KEYS,
  ACCESS_FEED_DAY_READS_KEYS,
  ACCESS_FEED_DAY_VIEWS_KEYS,
  ACCESS_FEED_KEYS,
  ACCESS_FEED_API_KEY_DAY_REQUESTS_KEYS,
  ACCESS_FEED_PEER_DAY_READS_KEYS,
  ACCESS_FEED_PEER_DAY_VIEWS_KEYS,
  COACH_READ_LOG_KEYS,
  accessFeedCredentialKey,
  coachReadClassKey,
} from '@pierre/shared-constants';
import {
  groupAccessFeedApiKeyDays,
  groupAccessFeedApps,
  revokedInKeySettings,
  type AccessFeedApiKeyGroup,
  type AccessFeedAppGroup,
} from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import { Button, ConfirmDialog, EmptyState, Section } from '../ui';
import { useAccessFeed, useConnectedApps, useRevokeConnectedApps } from '../../hooks/useAccessFeed';

/**
 * One read an app made, and when: a tool it called, or a turn of Dravr's
 * agent it asked for, which chose its own tools and so names none.
 */
function AppReadRow({ read }: { read: AccessFeedAppRead }) {
  const { t, language } = useTranslation();
  const words =
    read.kind === 'agent_turn' || read.tool === null
      ? t(ACCESS_FEED_KEYS.agentTurnRow)
      : t(ACCESS_FEED_KEYS.appRow, { tool: read.tool });
  return (
    <li
      className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5 py-1"
      data-testid="access-feed-read"
      data-kind={read.kind}
      data-tool={read.tool ?? undefined}
    >
      <span className="min-w-0 text-sm text-on-surface-variant break-words" data-testid="access-feed-read-tool">
        {words}
      </span>
      <time className="text-xs text-outline" dateTime={read.read_at}>
        {formatDateTime(read.read_at, language)}
      </time>
    </li>
  );
}

interface AppGroupProps {
  group: AccessFeedAppGroup;
  revoking: boolean;
  onRevoke: (group: AccessFeedAppGroup) => void;
}

/**
 * One outside app and its calls. Support's calls are one group that names no
 * client, since the client is the admin's. Revoke shows only for a Connected
 * app the athlete still holds a grant of.
 */
function AppGroup({ group, revoking, onRevoke }: AppGroupProps) {
  const { t } = useTranslation();
  const credentialKey = accessFeedCredentialKey(group.credential);
  const meta = [
    group.host,
    credentialKey && t(credentialKey),
    group.transport === 'a2a' ? t(ACCESS_FEED_KEYS.overA2a) : null,
  ].filter(Boolean);
  return (
    <li
      className="border-t ghost-border-faint py-3 first:border-t-0"
      data-testid={group.kind === 'support' ? 'access-feed-support' : 'access-feed-app'}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          {group.kind === 'support' ? (
            <p className="text-sm font-medium text-on-surface">{t(ACCESS_FEED_KEYS.supportRead)}</p>
          ) : (
            <>
              <p className="text-sm font-medium text-on-surface break-all" data-testid="access-feed-app-name">
                {group.name ?? t(ACCESS_FEED_KEYS.unknownApp)}
              </p>
              {meta.length > 0 && <p className="text-xs text-on-surface-variant break-words">{meta.join(' · ')}</p>}
            </>
          )}
        </div>
        {group.revokeGrantIds.length > 0 && (
          <Button
            variant="danger-outline"
            size="sm"
            className="flex-shrink-0"
            disabled={revoking}
            onClick={() => onRevoke(group)}
            data-testid="access-feed-revoke"
          >
            {t('app.revoke')}
          </Button>
        )}
      </div>
      {group.kind === 'app' && revokedInKeySettings(group.credential) && (
        <p className="mt-1 text-xs text-on-surface-variant" data-testid="access-feed-key-hint">
          {t(ACCESS_FEED_KEYS.apiKeyHint)}
        </p>
      )}
      <ul className="mt-1">
        {group.reads.map((read, index) => (
          <AppReadRow key={`${read.read_at}-${read.tool}-${index}`} read={read} />
        ))}
      </ul>
    </li>
  );
}

/**
 * One of the athlete's API keys and the days its REST requests read their
 * data. It is managed in the API-keys settings, so it carries the hint there
 * and no Revoke.
 */
function ApiKeyGroup({ group }: { group: AccessFeedApiKeyGroup }) {
  const { t, language } = useTranslation();
  return (
    <li className="border-t ghost-border-faint py-3 first:border-t-0" data-testid="access-feed-api-key">
      <p className="text-sm font-medium text-on-surface break-all" data-testid="access-feed-api-key-name">
        {group.name}
      </p>
      <p className="text-xs text-on-surface-variant break-words">
        <span className="font-mono">{group.prefix}</span>
        {' · '}
        {t(accessFeedCredentialKey('api_key'))}
      </p>
      <p className="mt-1 text-xs text-on-surface-variant" data-testid="access-feed-key-hint">
        {t(ACCESS_FEED_KEYS.apiKeyHint)}
      </p>
      <ul className="mt-1">
        {group.days.map((day) => (
          <li
            key={day.day}
            className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5 py-1"
            data-testid="access-feed-api-key-day"
          >
            <span className="text-sm text-on-surface-variant" data-testid="access-feed-api-key-requests">
              {t(countKey(day.requests, language, ACCESS_FEED_API_KEY_DAY_REQUESTS_KEYS), { count: day.requests })}
            </span>
            <time className="text-xs text-outline" dateTime={day.day}>
              {formatDate(day.day, language, 'UTC')}
            </time>
          </li>
        ))}
      </ul>
    </li>
  );
}

/**
 * One day of the coach's reads in one group: who, which kind of data, how
 * many reads and group views, and whether an AI tool the coach connects got
 * them or Dravr support made them.
 */
function CoachDayRow({ day }: { day: AccessFeedCoachDay }) {
  const { t, language } = useTranslation();
  const group = day.group_name ?? t(ACCESS_FEED_COACH_KEYS.unknownGroup);
  const counts = [
    day.reads > 0 ? t(countKey(day.reads, language, ACCESS_FEED_DAY_READS_KEYS), { count: day.reads, group }) : null,
    day.views > 0 ? t(countKey(day.views, language, ACCESS_FEED_DAY_VIEWS_KEYS), { count: day.views, group }) : null,
  ].filter(Boolean);
  return (
    <li className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5 py-1.5" data-testid="access-feed-coach-day">
      <span className="text-sm text-on-surface">
        <span className="font-medium" data-testid="access-feed-coach-name">
          {day.coach_name ?? t(COACH_READ_LOG_KEYS.formerCoach)}
        </span>
        {' · '}
        <span>{t(coachReadClassKey(day.data_class))}</span>
      </span>
      <time className="text-xs text-outline" dateTime={day.day}>
        {formatDate(day.day, language, 'UTC')}
      </time>
      <span className="basis-full text-xs text-on-surface-variant" data-testid="access-feed-coach-counts">
        {counts.join(' · ')}
      </span>
      {day.support_read && (
        <span className="basis-full text-xs text-on-surface-variant" data-testid="access-feed-coach-support">
          {t(COACH_READ_LOG_KEYS.supportRead)}
        </span>
      )}
      {day.served_to_assistant && !day.support_read && (
        <span className="basis-full text-xs text-on-surface-variant" data-testid="access-feed-coach-served">
          {t(COACH_READ_LOG_KEYS.servedToAssistant)}
        </span>
      )}
    </li>
  );
}

/**
 * One day of a fellow group member's reads in one group, under the athlete's
 * peer sharing: the sentence naming who read how often when the member's
 * agent read, else the member alone, then the day's group views, and
 * whether an AI tool the member connects got them or Dravr support made them.
 */
function PeerDayRow({ day }: { day: AccessFeedPeerDay }) {
  const { t, language } = useTranslation();
  const member = day.member_name ?? t(ACCESS_FEED_KEYS.formerMember);
  const group = day.group_name ?? t(ACCESS_FEED_COACH_KEYS.unknownGroup);
  const line =
    day.reads > 0
      ? t(countKey(day.reads, language, ACCESS_FEED_PEER_DAY_READS_KEYS), { member, count: day.reads, group })
      : member;
  return (
    <li className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5 py-1.5" data-testid="access-feed-peer-day">
      <span className="min-w-0 text-sm text-on-surface break-words" data-testid="access-feed-peer-line">
        {line}
      </span>
      <time className="text-xs text-outline" dateTime={day.day}>
        {formatDate(day.day, language, 'UTC')}
      </time>
      {day.views > 0 && (
        <span className="basis-full text-xs text-on-surface-variant" data-testid="access-feed-peer-views">
          {t(countKey(day.views, language, ACCESS_FEED_PEER_DAY_VIEWS_KEYS), { count: day.views, group })}
        </span>
      )}
      {day.support_read && (
        <span className="basis-full text-xs text-on-surface-variant" data-testid="access-feed-peer-support">
          {t(ACCESS_FEED_KEYS.peerSupportRead)}
        </span>
      )}
      {day.served_to_assistant && !day.support_read && (
        <span className="basis-full text-xs text-on-surface-variant" data-testid="access-feed-peer-served">
          {t(ACCESS_FEED_KEYS.peerServedToAssistant)}
        </span>
      )}
    </li>
  );
}

/**
 * The athlete's record of who read their data (carnet#1543): each call or
 * agent turn an outside app made over their own data, their API keys' days,
 * and their human coach's and fellow members' reads as daily counts, over the
 * window the server names.
 */
export default function AccessFeedSection() {
  const { t } = useTranslation();
  const { data: feed, isLoading, isError } = useAccessFeed();
  const { data: grants } = useConnectedApps();
  const revoke = useRevokeConnectedApps();
  const [toRevoke, setToRevoke] = useState<AccessFeedAppGroup | null>(null);

  const groups = useMemo(() => groupAccessFeedApps(feed?.app_reads ?? [], grants ?? []), [feed, grants]);
  const apiKeys = useMemo(() => groupAccessFeedApiKeyDays(feed?.api_key_days ?? []), [feed]);
  const coachDays = feed?.coach_days ?? [];
  const peerDays = feed?.peer_days ?? [];
  const nothingRead = groups.length === 0 && apiKeys.length === 0 && coachDays.length === 0 && peerDays.length === 0;
  const appName = toRevoke?.name ?? t(ACCESS_FEED_KEYS.unknownApp);

  return (
    <Section
      title={t(ACCESS_FEED_KEYS.title)}
      description={feed && t(ACCESS_FEED_KEYS.intro, { days: feed.days })}
      data-testid="access-feed"
    >
      {isError ? (
        <p className="text-sm text-error" role="alert" data-testid="access-feed-error">
          {t(ACCESS_FEED_KEYS.loadFailed)}
        </p>
      ) : isLoading || !feed ? null : nothingRead ? (
        <EmptyState data-testid="access-feed-empty">{t(ACCESS_FEED_KEYS.empty, { days: feed.days })}</EmptyState>
      ) : (
        <div className="space-y-6">
          {(groups.length > 0 || apiKeys.length > 0) && (
            <div>
              <h3 className="text-sm font-medium text-on-surface mb-1">{t(ACCESS_FEED_KEYS.appsHeading)}</h3>
              <ul data-testid="access-feed-apps">
                {groups.map((group) => (
                  <AppGroup
                    key={group.key}
                    group={group}
                    revoking={revoke.isPending}
                    onRevoke={setToRevoke}
                  />
                ))}
                {apiKeys.map((key) => (
                  <ApiKeyGroup key={key.keyId} group={key} />
                ))}
              </ul>
            </div>
          )}
          {coachDays.length > 0 && (
            <div>
              <h3 className="text-sm font-medium text-on-surface mb-1">{t(ACCESS_FEED_COACH_KEYS.feedHeading)}</h3>
              <ul className="divide-y divide-outline-variant/40" data-testid="access-feed-coach-days">
                {coachDays.map((day, index) => (
                  <CoachDayRow key={`${day.day}-${day.group_id}-${day.data_class}-${day.support_read}-${index}`} day={day} />
                ))}
              </ul>
            </div>
          )}
          {peerDays.length > 0 && (
            <div>
              <h3 className="text-sm font-medium text-on-surface mb-1">{t(ACCESS_FEED_KEYS.peersHeading)}</h3>
              <ul className="divide-y divide-outline-variant/40" data-testid="access-feed-peer-days">
                {peerDays.map((day, index) => (
                  <PeerDayRow key={`${day.day}-${day.group_id}-${day.member_name ?? ''}-${index}`} day={day} />
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
      <ConfirmDialog
        isOpen={toRevoke !== null}
        onClose={() => setToRevoke(null)}
        onConfirm={() =>
          toRevoke && revoke.mutate(toRevoke.revokeGrantIds, { onSuccess: () => setToRevoke(null) })
        }
        title={t('app.revokeAccessQ')}
        message={t('app.confirmRevokeAppAccess', { app: appName })}
        confirmLabel={t('app.revoke')}
        variant="danger"
        isLoading={revoke.isPending}
      />
    </Section>
  );
}
