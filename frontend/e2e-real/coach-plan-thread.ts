// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A fresh coach and athlete whose plan thread holds the coach's open draft, seeded by pierre-cli, and the athlete's season read back
// ABOUTME: Served by the Maestro control process (home-sync-control.ts) to the mobile coach plan-thread flow, a new pair on every attempt

import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { firstPartySignIn } from '../../scripts/auth/first-party-sign-in.js';

const execFileAsync = promisify(execFile);

/** One password for every account this makes: each is new, so nothing is shared but the string. */
const PASSWORD = 'PlanE2e-Pass1234';

/** The athlete's roster name, which the coach's thread is titled and subtitled with. */
const ATHLETE_NAME = 'Maya Tempo';

/** How long one `pierre-cli` run may take before the setup fails naming it. */
const CLI_TIMEOUT_MS = 60_000;

/** The athletes made so far, so a state read signs in as nobody else. */
const athletes = new Set<string>();

/** The server under test, the database it serves and the CLI that seeds it. */
export interface PlanServer {
  pierreUrl: string;
  /** The `pierre-cli` binary built from the same tree as the server. */
  cliPath: string | undefined;
  /** The SQLite file the server under test uses. */
  databasePath: string | undefined;
}

/** What a setup made, as the flow signs in with it and asserts it. */
export interface CoachPlanThread {
  coachEmail: string;
  coachPassword: string;
  athleteEmail: string;
  /** The athlete as the coach's thread names them. */
  athleteName: string;
  /** The coach's group thread, the row that reaches Group info. */
  groupName: string;
  /** The athlete's own season, which the draft has not replaced yet. */
  liveRace: string;
  /** The season the coach's draft re-lays, which the card shows and Publish makes live. */
  draftRace: string;
}

/** What the athlete's live plan is after the flow, as the athlete reads it. */
export interface AthletePlanState {
  /** The live season's goal race, or `null` when the athlete has none. */
  liveRace: string | null;
}

/**
 * Run `pierre-cli` against the server's database. The CLI's own error output
 * names what failed; the arguments are never echoed, since they carry a
 * password.
 */
async function cli(server: PlanServer, args: string[]): Promise<void> {
  if (server.cliPath === undefined || server.cliPath === '') {
    throw new Error('PIERRE_CLI names the pierre-cli binary that seeds the server under test');
  }
  if (server.databasePath === undefined || server.databasePath === '') {
    throw new Error('E2E_REAL_DATABASE_PATH names the SQLite file the server under test uses');
  }
  try {
    await execFileAsync(server.cliPath, args, {
      env: { ...process.env, DATABASE_URL: `sqlite:${server.databasePath}`, RUST_LOG: 'warn' },
      timeout: CLI_TIMEOUT_MS,
    });
  } catch (error) {
    const failed = error as { stderr?: string; code?: number | string };
    const why = failed.stderr?.trim() || `exit ${String(failed.code)}`;
    throw new Error(`pierre-cli ${args.slice(0, 2).join(' ')} failed: ${why}`, { cause: error });
  }
}

async function call(server: PlanServer, path: string, token: string): Promise<unknown> {
  const response = await fetch(`${server.pierreUrl}${path}`, { headers: { Authorization: `Bearer ${token}` } });
  if (!response.ok) {
    throw new Error(`${path} answered ${response.status}: ${await response.text()}`);
  }
  return response.json();
}

async function signIn(server: PlanServer, email: string): Promise<string> {
  const tokens = await firstPartySignIn({ baseUrl: server.pierreUrl, email, password: PASSWORD });
  return tokens.access_token;
}

interface ListedThread {
  id: string;
  title: string;
  group_id?: string | null;
  subject_user_id?: string | null;
  subject_display_name?: string | null;
}

interface SubjectPlanRead {
  live: { goal_race: { name: string } } | null;
  draft: { goal_race: { name: string } } | null;
  draft_state: { conflicts: unknown[]; season_relaid: boolean } | null;
}

/**
 * A new coach and athlete, made with `pierre-cli user create` the way the CI
 * lanes make their accounts, and seeded with `pierre-cli seed
 * coach-plan-thread` (crates/pierre-seeders/src/coach_plan_thread.rs): the
 * group the coach coaches the athlete in, the athlete's live season, the
 * coach's thread about them and the card of the coach's open draft.
 *
 * Every call makes a new pair. The flow publishes the draft, which consumes
 * it, and a retried attempt — the iOS lane re-runs a whole batch — must find
 * a draft still to publish.
 *
 * What the flow asserts is read back from the server as the coach, not
 * restated here: the thread about the athlete and the name it carries, the
 * group thread's title, and the subject-plan read with an open draft that
 * overwrites nothing, which is the premise of the card's Publish.
 */
export async function freshCoachPlanThread(server: PlanServer): Promise<CoachPlanThread> {
  const stamp = `${Date.now()}-${Math.floor(Math.random() * 1e6)}`;
  const coachEmail = `plan-coach-e2e-${stamp}@pierre.dev`;
  const athleteEmail = `plan-athlete-e2e-${stamp}@pierre.dev`;
  await cli(server, ['user', 'create', '--email', coachEmail, '--password', PASSWORD, '--name', 'Coach e2e']);
  await cli(server, ['user', 'create', '--email', athleteEmail, '--password', PASSWORD, '--name', ATHLETE_NAME]);
  await cli(server, ['seed', 'coach-plan-thread', '--coach-email', coachEmail, '--athlete-email', athleteEmail]);

  const token = await signIn(server, coachEmail);
  const { conversations } = (await call(server, '/api/chat/conversations?limit=50&offset=0', token)) as {
    conversations: ListedThread[];
  };
  const planThread = conversations.find((thread) => thread.subject_user_id);
  const groupThread = conversations.find((thread) => thread.group_id && !thread.subject_user_id);
  if (planThread === undefined || groupThread === undefined || !planThread.subject_display_name) {
    throw new Error(`the coach has no group thread and thread about the athlete: ${JSON.stringify(conversations)}`);
  }
  const read = (await call(server, `/api/chat/conversations/${planThread.id}/subject-plan`, token)) as SubjectPlanRead;
  if (
    read.live === null ||
    read.draft === null ||
    read.draft_state === null ||
    read.draft_state.conflicts.length > 0 ||
    read.draft_state.season_relaid
  ) {
    throw new Error(`the coach holds no draft that publishes cleanly: ${JSON.stringify(read)}`);
  }
  athletes.add(athleteEmail);
  return {
    coachEmail,
    coachPassword: PASSWORD,
    athleteEmail,
    athleteName: planThread.subject_display_name,
    groupName: groupThread.title,
    liveRace: read.live.goal_race.name,
    draftRace: read.draft.goal_race.name,
  };
}

/** The athlete's live season as the athlete reads it, after the flow published to it. */
export async function athletePlanState(server: PlanServer, athleteEmail: string): Promise<AthletePlanState> {
  if (!athletes.has(athleteEmail)) {
    throw new Error(`${athleteEmail} is not an athlete this control made`);
  }
  const token = await signIn(server, athleteEmail);
  const { plan } = (await call(server, '/api/me/training-plan', token)) as {
    plan: { goal_race: { name: string } } | null;
  };
  return { liveRace: plan?.goal_race.name ?? null };
}
