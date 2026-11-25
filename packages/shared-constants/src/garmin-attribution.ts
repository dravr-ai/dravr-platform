// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The line Garmin's brand guidelines owe a figure derived from Garmin-sourced data, in the reader's language
// ABOUTME: The server says what Garmin contributed; both clients print this line beneath the figure's title, never in a tooltip

import type { GarminContribution } from '@pierre/shared-types';
import { GARMIN_CONNECT_BRAND } from './brands';
import { connectPlan, type ConnectPlanCard } from './connect-plan';

/**
 * The corpus key for each contribution (carnet#1154). Garmin's guidelines ask
 * a derived figure to name Garmin as its distinct or contributing source,
 * whichever is true, without implying Garmin stands behind the other sources'
 * data; the words follow the acceptable samples they give.
 */
export const GARMIN_CONTRIBUTION_KEY: Record<GarminContribution, string> = {
  sole: 'garminAttribution.sole',
  part: 'garminAttribution.part',
};

/** The shape of `t()` this module needs. */
type Translate = (key: string) => string;

/**
 * The line a figure carries directly beneath its title, or `undefined` when
 * no Garmin-sourced data went into it.
 */
export function garminContributionLine(
  t: Translate,
  contribution: GarminContribution | null | undefined,
): string | undefined {
  return contribution ? t(GARMIN_CONTRIBUTION_KEY[contribution]) : undefined;
}

/** The provider Garmin's own OAuth connection is authorized and stored under. */
const GARMIN_CONNECT_PROVIDER = 'garmin';

/**
 * Where the web client serves the official Garmin Connect app tile, copied
 * from Garmin's brand assets and only scaled.
 */
export const GARMIN_CONNECT_TILE_WEB_PATH = '/brand/garmin-connect-app.png';

/**
 * Whether a connection card stands for Garmin's official API (carnet#1154):
 * connected through Garmin's OAuth, or, unconnected, connecting through it.
 * The Garmin card is one card over two backends, so the same card that
 * connects through the account sign-in, or is connected through it, is not:
 * Garmin's guidelines speak to the official integration, and only it wears
 * Garmin Connect's name and tile.
 */
export function isGarminConnectCard(card: ConnectPlanCard): boolean {
  if (card.connected) return card.connected_backend === GARMIN_CONNECT_PROVIDER;
  const primary = connectPlan(card).primary;
  return primary.kind === 'oauth' && primary.provider === GARMIN_CONNECT_PROVIDER;
}

/** The name a connection card shows: Garmin Connect in full on Garmin's OAuth card, the served name elsewhere. */
export function connectCardName(card: ConnectPlanCard & { display_name: string }): string {
  return isGarminConnectCard(card) ? GARMIN_CONNECT_BRAND : card.display_name;
}
