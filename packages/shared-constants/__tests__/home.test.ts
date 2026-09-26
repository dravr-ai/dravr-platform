// ABOUTME: Unit tests for the Home surface's shared declarations — its registry row, query keys and refetch delay
// ABOUTME: Pins Home as the first destination on both platforms and the key shapes both clients cache under

import { describe, it, expect } from 'vitest';
import {
  HOME_STALE_REFETCH_DELAY_MS,
  IDLE_STOP_AFTER_MS,
  QUERY_KEYS,
  USER_SURFACES,
  surfaceById,
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

describe('home stale refetch delay', () => {
  it('asks again well before the client would go idle', () => {
    expect(HOME_STALE_REFETCH_DELAY_MS).toBe(15000);
    expect(HOME_STALE_REFETCH_DELAY_MS).toBeLessThan(IDLE_STOP_AFTER_MS);
  });
});
