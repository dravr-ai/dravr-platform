// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's Home — "Mes athlètes": À traiter, the team table and the season, behind the switch that returns to their own training
// ABOUTME: One roster read feeds the three views; every figure states its scope, form is the server's band, and a non-sharer shows status only

import { useMemo, useState } from 'react';
import { clsx } from 'clsx';
import { useTranslation } from '@pierre/i18n';
import { COACH_SCOPE_KEYS, countKey } from '@pierre/shared-constants';
import { athletesInGroup, formatInstant, localToday } from '@pierre/ui-logic';
import { useIsMobile } from '../../hooks/useBreakpoint';
import { useCoachRoster } from '../../hooks/useCoachRoster';
import { EmptyState, SearchField, Select } from '../ui';
import { CoachHomeSwitch } from './CoachHomeSwitch';
import { CoachQueueView } from './CoachQueueView';
import { CoachRosterTable } from './CoachRosterTable';
import { CoachSeasonView } from './CoachSeasonView';
import { coachHomeRoute, type CoachHomeRoute } from './coachHomeRoute';
import { useCoachDestinations } from './useCoachDestinations';

interface CoachHomeProps {
  route: Exclude<CoachHomeRoute, { view: 'me' }>;
  /** Dashboard route navigator, `tab[/subview]`. */
  onNavigate: (route: string) => void;
}

const VIEWS = [
  { id: 'queue', labelKey: 'coachHome.views.queue', route: { view: 'queue', athleteId: null } },
  { id: 'team', labelKey: 'coachHome.views.roster', route: { view: 'team' } },
  { id: 'season', labelKey: 'coachHome.views.season', route: { view: 'season' } },
] as const satisfies readonly { id: string; labelKey: string; route: CoachHomeRoute }[];

export default function CoachHome({ route, onNavigate }: CoachHomeProps) {
  const { t, language } = useTranslation();
  const isMobile = useIsMobile();
  const { roster, isLoading, isError, readAt, refetch } = useCoachRoster(true);
  const destinations = useCoachDestinations(onNavigate);
  const [groupId, setGroupId] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const today = useMemo(() => localToday(), []);

  const athletes = useMemo(() => athletesInGroup(roster?.athletes ?? [], groupId), [roster, groupId]);

  const scopeLine = useMemo(() => {
    if (roster === undefined) return '';
    const { visible_members: visible, roster_members: total } = roster.scope;
    const line = t(countKey(visible, language, COACH_SCOPE_KEYS), { visible, total });
    if (readAt <= 0) return line;
    const time = formatInstant(new Date(readAt).toISOString(), language, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
    return `${line} · ${t('coachHome.readAt', { time })}`;
  }, [roster, readAt, language, t]);

  const go = (next: CoachHomeRoute) => onNavigate(coachHomeRoute(next));
  const openAthlete = (athleteId: string | null) => go({ view: 'queue', athleteId });

  let body: React.ReactNode;
  if (isLoading) {
    body = (
      <div className="flex flex-1 items-center justify-center" role="status" aria-label={t('coachHome.loading')}>
        <div className="pierre-spinner" />
      </div>
    );
  } else if (isError || roster === undefined) {
    body = (
      <div className="px-6 py-6" data-testid="coach-home-failed">
        <EmptyState action={{ label: t('common.retry'), onClick: refetch }}>{t('coachHome.loadFailed')}</EmptyState>
      </div>
    );
  } else if (roster.athletes.length === 0) {
    body = (
      <div className="px-6 py-6">
        <EmptyState data-testid="coach-home-empty">{t('coachHome.noAthletes')}</EmptyState>
      </div>
    );
  } else if (route.view === 'queue') {
    body = (
      <CoachQueueView
        athletes={athletes}
        selectedId={route.athleteId}
        onSelect={openAthlete}
        onSeeAll={() => go({ view: 'team' })}
        today={today}
        destinations={destinations}
      />
    );
  } else if (route.view === 'team') {
    body = (
      <CoachRosterTable
        athletes={athletes}
        groups={roster.groups}
        total={roster.athletes.length}
        groupId={groupId}
        onGroup={setGroupId}
        search={search}
        today={today}
        onOpen={(id) => openAthlete(id)}
        onBack={() => openAthlete(null)}
        destinations={destinations}
      />
    );
  } else {
    body = <CoachSeasonView athletes={athletes} today={today} onOpen={(id) => openAthlete(id)} />;
  }

  return (
    <div className="flex h-full min-h-0 flex-col bg-surface" data-testid="coach-home" data-view={route.view}>
      <header className="flex min-h-[52px] shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b ghost-border px-4 py-2">
        <CoachHomeSwitch scope="athletes" onChange={() => onNavigate('home/me')} stretch={isMobile} />
        {scopeLine !== '' && (
          <span className="text-xs text-on-surface-variant" data-testid="coach-scope">
            {scopeLine}
          </span>
        )}
        <nav aria-label={t('coachHome.views.aria')} className="flex items-center gap-1 md:ml-auto" data-testid="coach-views">
          {VIEWS.map((view) => (
            <button
              key={view.id}
              type="button"
              aria-current={route.view === view.id ? 'page' : undefined}
              data-testid={`coach-view-${view.id}`}
              onClick={() => go(view.route)}
              className={clsx(
                'h-8 rounded-lg px-2.5 text-sm focus-ring-visible',
                route.view === view.id
                  ? 'bg-primary-container font-semibold text-on-primary-container'
                  : 'font-medium text-on-surface-variant hover:bg-surface-container-low',
              )}
            >
              {t(view.labelKey)}
            </button>
          ))}
        </nav>
        {roster !== undefined && roster.groups.length > 1 && route.view !== 'team' && (
          <div className="w-44">
            <Select
              size="sm"
              aria-label={t('coachHome.queue.groupFilter')}
              data-testid="coach-group-filter"
              value={groupId ?? ''}
              onChange={(event) => setGroupId(event.target.value === '' ? null : event.target.value)}
              options={[
                { value: '', label: t('coachHome.queue.groupFilter') },
                ...roster.groups.map((group) => ({ value: group.id, label: group.name })),
              ]}
            />
          </div>
        )}
        {route.view === 'team' && (
          <SearchField
            aria-label={t('coachHome.roster.search')}
            placeholder={t('coachHome.roster.search')}
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            className="w-full md:w-[220px]"
            data-testid="coach-roster-search"
          />
        )}
      </header>
      {body}
    </div>
  );
}
