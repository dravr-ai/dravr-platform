// ABOUTME: Unit tests for the Home surface's shared declarations — its registry row, query keys and refetch delay
// ABOUTME: Pins Home as the first destination on both platforms and the key shapes both clients cache under

import { describe, it, expect } from 'vitest';
import {
  HOME_STALE_REFETCH_DELAYS_MS,
  IDLE_STOP_AFTER_MS,
  QUERY_KEYS,
  USER_SURFACES,
  surfaceById,
  sportHasRoutes,
  surfacesFor,
  webNavLabels,
} from '../src';

describe('home surface', () => {
  it('is declared on both platforms with a sidebar label', () => {
    const home = surfaceById('home');
    expect(home).toEqual({
      id: 'home',
      label: 'Home',
      web: 'home',
      mobile: '/(app)/(tabs)/(home)',
      webNav: 'Home',
      blocks: [],
    });
    expect(surfacesFor('web').some((surface) => surface.id === 'home')).toBe(true);
    expect(surfacesFor('mobile').some((surface) => surface.id === 'home')).toBe(true);
  });

  it('comes first, because it is where an athlete lands', () => {
    expect(USER_SURFACES[0].id).toBe('home');
    expect(webNavLabels()[0]).toBe('Home');
  });
});

describe('home query keys', () => {
  it('scopes every Home read under one prefix, so one invalidation refreshes the page', () => {
    const keys = [
      QUERY_KEYS.home.recentActivities(),
      QUERY_KEYS.home.activityRoute('strava', '12849301144'),
      QUERY_KEYS.home.trainingPlan('fr'),
    ];
    for (const key of keys) {
      expect(key.slice(0, QUERY_KEYS.home.all.length)).toEqual([...QUERY_KEYS.home.all]);
    }
  });

  it('keys the default activity list apart from an explicit limit', () => {
    expect(QUERY_KEYS.home.recentActivities()).toEqual(['home', 'recent-activities', null]);
    expect(QUERY_KEYS.home.recentActivities(10)).toEqual(['home', 'recent-activities', 10]);
  });

  it('keys a route by provider as well as id, since ids are only unique per provider', () => {
    expect(QUERY_KEYS.home.activityRoute('strava', '1001')).not.toEqual(
      QUERY_KEYS.home.activityRoute('garmin', '1001'),
    );
  });

  it('keys the plan by locale, since the server names the flavour in it', () => {
    expect(QUERY_KEYS.home.trainingPlan('fr')).toEqual(['home', 'training-plan', 'fr']);
    expect(QUERY_KEYS.home.trainingPlan('en')).not.toEqual(QUERY_KEYS.home.trainingPlan('fr'));
  });
});

describe('home stale refetch schedule', () => {
  /** The server's bound on one background refresh, `REVALIDATION_TIMEOUT_SECS`. */
  const SERVER_REFRESH_BOUND_MS = 240000;
  const total = HOME_STALE_REFETCH_DELAYS_MS.reduce((sum, delay) => sum + delay, 0);

  it('asks first after fifteen seconds, then at widening waits', () => {
    expect(HOME_STALE_REFETCH_DELAYS_MS).toEqual([15000, 30000, 60000, 150000]);
  });

  it('asks last after the server has given up on a refresh', () => {
    expect(total).toBeGreaterThan(SERVER_REFRESH_BOUND_MS);
  });

  it('ends before the client would go idle', () => {
    expect(total).toBeLessThan(IDLE_STOP_AFTER_MS);
  });
});

describe('sportHasRoutes', () => {
  it('says yes for the sports the route search covers, however the wire spells them', () => {
    expect(sportHasRoutes('run')).toBe(true);
    expect(sportHasRoutes('ride')).toBe(true);
    expect(sportHasRoutes('Trail Run')).toBe(true);
    expect(sportHasRoutes('nordic_ski')).toBe(true);
  });

  it('says no for a session with no route, and for a sport it does not know', () => {
    expect(sportHasRoutes('swim')).toBe(false);
    expect(sportHasRoutes('strength_training')).toBe(false);
    expect(sportHasRoutes('virtual_ride')).toBe(false);
    expect(sportHasRoutes('rest')).toBe(false);
    expect(sportHasRoutes('underwater hockey')).toBe(false);
  });
});
