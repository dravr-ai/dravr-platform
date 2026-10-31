// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The singular and plural catalogue keys of every count the web and mobile clients print, for countKey to choose between
// ABOUTME: One object per count, so web and mobile cannot pick the form differently (carnet#1212)
//
// chat-utils sits below this package and keeps its own folded-turn pair beside
// TURN_TRAIL_KEYS; every other count lives here.

import type { CountKeys } from '@pierre/chat-utils';

/** How many athletes installed a store agent: « 1 utilisateur », « 0 utilisateur », « 3 utilisateurs ». */
export const INSTALL_COUNT_KEYS: CountKeys = {
  one: 'discover.installCountOne',
  many: 'discover.installCountN',
};

/** How many facts the agent remembers: « 1 info », « 4 infos ». */
export const MEMORY_FACT_COUNT_KEYS: CountKeys = {
  one: 'shell.memoryFactCountOne',
  many: 'shell.memoryFactCountN',
};

/** The notification bell's spoken label: « Notifications, 1 non lue », « Notifications, 3 non lues ». */
export const NOTIFICATIONS_BELL_UNREAD_KEYS: CountKeys = {
  one: 'shell.notificationsBellUnreadOne',
  many: 'shell.notificationsBellUnreadN',
};

/** The notification panel's unread line. */
export const NOTIFICATIONS_UNREAD_KEYS: CountKeys = {
  one: 'shell.notificationsUnreadOne',
  many: 'shell.notificationsUnreadN',
};

/** How many notifications a collapsed one stands for: « +1 similaire », « +3 similaires ». */
export const COLLAPSED_SIMILAR_KEYS: CountKeys = {
  one: 'notifications.collapsedSimilarOne',
  many: 'notifications.collapsedSimilarN',
};

/** A notification category's daily cap: « 1 par jour », « 5 par jour ». */
export const PER_DAY_KEYS: CountKeys = {
  one: 'frag.perDayOne',
  many: 'frag.perDayN',
};

/**
 * « Strava connecté » / « Strava, Garmin connectés »: the participle agrees
 * with how many providers are named. Its forms interpolate `{{providers}}`,
 * the names themselves, rather than a count.
 */
export const PROVIDERS_CONNECTED_KEYS: CountKeys = {
  one: 'chat.providersConnectedOne',
  many: 'chat.providersConnectedN',
};

/** How many activities a gear pair carries: « 0 activité », « 12 activités ». */
export const GEAR_ACTIVITY_COUNT_KEYS: CountKeys = {
  one: 'gear.activityCountOne',
  many: 'gear.activityCountN',
};

/** How many activities the Home volume card's week holds: « 0 activité », « 3 activités ». */
export const WEEK_ACTIVITY_COUNT_KEYS: CountKeys = {
  one: 'home.volume.activityCountOne',
  many: 'home.volume.activityCountN',
};

/** The verdict chip's tooltip with an evidence strength: « 1 verdict sur les affirmations — … ». */
export const CLAIM_VERDICT_SUMMARY_KEYS: CountKeys = {
  one: 'app.claimVerdictSummaryOne',
  many: 'app.claimVerdictSummaryN',
};

/** The verdict chip's tooltip when no row carries an evidence strength. */
export const CLAIM_VERDICT_SUMMARY_STATUS_ONLY_KEYS: CountKeys = {
  one: 'app.claimVerdictSummaryStatusOnlyOne',
  many: 'app.claimVerdictSummaryStatusOnlyN',
};

/** A conversation row's spoken label with its unread count (mobile). */
export const OPEN_ROW_UNREAD_KEYS: CountKeys = {
  one: 'app.openRowUnreadOne',
  many: 'app.openRowUnreadN',
};

/** A conversation row's unread badge, spoken (web): « 1 non lu », « 3 non lus ». */
export const UNREAD_COUNT_ARIA_KEYS: CountKeys = {
  one: 'chat.unreadCountAriaOne',
  many: 'chat.unreadCountAriaN',
};

/** The weeks a plan card leaves out: « 1 semaine de plus dans le plan ». */
export const PLAN_MORE_WEEKS_KEYS: CountKeys = {
  one: 'plan.card.moreWeeksOne',
  many: 'plan.card.moreWeeksN',
};

/** A plan phase's ceiling on hard sessions per week: « 1 dure/sem », « 2 dures/sem ». */
export const PLAN_HARD_PER_WEEK_KEYS: CountKeys = {
  one: 'plan.card.hardPerWeekOne',
  many: 'plan.card.hardPerWeekN',
};

/** An API key's usage over the window: « 1 requête ces 30 derniers jours ». */
export const API_KEY_REQUESTS_KEYS: CountKeys = {
  one: 'apiKeys.requests30dOne',
  many: 'apiKeys.requests30dN',
};

/** The active AI-client tokens an athlete holds. */
export const ACTIVE_TOKEN_COUNT_KEYS: CountKeys = {
  one: 'tokens.activeCountOne',
  many: 'tokens.activeCountN',
};

/** The A2A apps an athlete connected. */
export const A2A_TOTAL_APPS_KEYS: CountKeys = {
  one: 'a2a.totalAppsCountOne',
  many: 'a2a.totalAppsCountN',
};

/** The onboarding proposal's summary of the activities found over its window. */
export const ONBOARDING_WINDOW_SUMMARY_KEYS: CountKeys = {
  one: 'app.obWindowSummaryOne',
  many: 'app.obWindowSummaryN',
};

/** A coach's weekly review: replies flagged this week. */
export const WEEK_FLAGGED_KEYS: CountKeys = {
  one: 'coachReview.weekFlaggedOne',
  many: 'coachReview.weekFlaggedN',
};

/** A coach's weekly review: replies corrected this week. */
export const WEEK_CORRECTED_KEYS: CountKeys = {
  one: 'coachReview.weekCorrectedOne',
  many: 'coachReview.weekCorrectedN',
};

/** The provider card's line for history imported from an export. */
export const IMPORT_HISTORY_ON_CARD_KEYS: CountKeys = {
  one: 'providers.import.historyOnCardOne',
  many: 'providers.import.historyOnCardN',
};

/** A completed import's state line: « 1 activité importée · … ». */
export const IMPORT_COMPLETED_KEYS: CountKeys = {
  one: 'providers.import.state.completedOne',
  many: 'providers.import.state.completedN',
};

/** A failed or cancelled import's state line: « Import arrêté · 1 importée ». */
export const IMPORT_STOPPED_KEYS: CountKeys = {
  one: 'providers.import.state.stoppedOne',
  many: 'providers.import.state.stoppedN',
};

/** The import screen's history line: « 1 activité · importée le … ». */
export const IMPORT_HISTORY_LINE_KEYS: CountKeys = {
  one: 'providers.import.strava.historyLineOne',
  many: 'providers.import.strava.historyLineN',
};

/** The body of the confirmation before imported history is deleted. */
export const IMPORT_DELETE_CONFIRM_BODY_KEYS: CountKeys = {
  one: 'providers.import.strava.deleteConfirmBodyOne',
  many: 'providers.import.strava.deleteConfirmBodyN',
};

/** The confirmation's action: « Supprimer 1 activité ». */
export const IMPORT_DELETE_CONFIRM_ACTION_KEYS: CountKeys = {
  one: 'providers.import.strava.deleteConfirmActionOne',
  many: 'providers.import.strava.deleteConfirmActionN',
};
