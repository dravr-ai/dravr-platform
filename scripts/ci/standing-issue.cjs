// ABOUTME: The one find-or-create for a monitor's standing issue — look it up across every page, then update, comment or close it
// ABOUTME: Required from actions/github-script steps (`require('./scripts/ci/standing-issue.cjs')`); pinned by standing-issue.test.sh
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// A standing issue is one issue per recurring condition, not one per run: the
// monitor finds the open issue it filed before and updates or comments on it,
// and files a fresh one only when none is open. Everything turns on the lookup,
// so there is exactly one.
//
// The lookup reads EVERY page. `issues.listForRepo` returns newest-first, and a
// standing issue's created_at never advances, so a lookup that reads one page
// stops seeing its own issue once enough newer issues carry the label — and from
// then on every run files a duplicate. A label like `dependencies` or
// `dravr-platform` is shared with everything else in its tracker, so a one-page
// lookup going blind is a matter of time, not of luck. Paging costs one request
// per hundred issues carrying the label.
//
// Pull requests share the issues endpoint and are skipped: Dependabot labels its
// pull requests `dependencies`, and a monitor must never rewrite one's body.
//
// Callers pass the `github` client github-script gives them, so the token that
// client holds (the default one, or DRAVR_CARNET_TOKEN for dravr-carnet) is the
// token every call here uses.

'use strict';

const matcher = ({ title, titlePrefixes }) => {
  if ((title === undefined) === (titlePrefixes === undefined)) {
    throw new Error('standing-issue: give exactly one of `title` or `titlePrefixes`');
  }
  if (title !== undefined) return (issue) => issue.title === title;
  return (issue) => titlePrefixes.some((prefix) => issue.title.startsWith(prefix));
};

// Every issue carrying `label` in `state` whose title is `title`, or starts with
// one of `titlePrefixes`, newest first.
async function findStandingIssues(github, { owner, repo, label, state = 'open', title, titlePrefixes }) {
  const matches = matcher({ title, titlePrefixes });
  const issues = await github.paginate(github.rest.issues.listForRepo, {
    owner, repo, state, labels: label, per_page: 100,
  });
  return issues.filter((issue) => !issue.pull_request && matches(issue));
}

// Creates `name` when the repository lacks it. Only a 404 means "absent": any
// other failure is rethrown rather than answered with a create that would fail
// on its own and hide the first error.
async function ensureLabel(github, { owner, repo, name, color, description }) {
  try {
    await github.rest.issues.getLabel({ owner, repo, name });
  } catch (error) {
    if (error.status !== 404) throw error;
    await github.rest.issues.createLabel({ owner, repo, name, color, description });
  }
}

// Finds the open standing issue and either replaces its title and body
// (`onExisting: 'update'`) or appends `body` as a comment (`onExisting:
// 'comment'`); files it with `labels` when none is open. `titlePrefixes` finds an
// issue whose title carries detail that changes between runs, and `title` is what
// it is (re)titled to. `newLabel` ({ color, description }) creates the lookup
// label before filing, for a repository that may not have it yet.
async function upsertStandingIssue(github, core, {
  owner, repo, label, title, titlePrefixes, body, onExisting, labels = [label], newLabel,
}) {
  if (onExisting !== 'update' && onExisting !== 'comment') {
    throw new Error(`standing-issue: onExisting must be 'update' or 'comment', got ${onExisting}`);
  }
  const lookup = titlePrefixes === undefined ? { title } : { titlePrefixes };
  const [existing] = await findStandingIssues(github, { owner, repo, label, ...lookup });

  if (!existing) {
    if (newLabel) await ensureLabel(github, { owner, repo, name: label, ...newLabel });
    const { data: created } = await github.rest.issues.create({ owner, repo, title, body, labels });
    core.info(`Opened issue #${created.number}`);
    return { number: created.number, created: true };
  }
  if (onExisting === 'update') {
    await github.rest.issues.update({ owner, repo, issue_number: existing.number, title, body });
    core.info(`Updated existing issue #${existing.number}`);
  } else {
    await github.rest.issues.createComment({ owner, repo, issue_number: existing.number, body });
    core.info(`Commented on existing issue #${existing.number}`);
  }
  return { number: existing.number, created: false };
}

// Comments `comment` on every open standing issue that matches, then closes it as
// completed. Returns the numbers it closed.
async function closeStandingIssues(github, core, { owner, repo, label, title, titlePrefixes, comment }) {
  const open = await findStandingIssues(github, { owner, repo, label, title, titlePrefixes });
  if (open.length === 0) core.info('No open standing issue to close');
  for (const issue of open) {
    await github.rest.issues.createComment({ owner, repo, issue_number: issue.number, body: comment });
    await github.rest.issues.update({
      owner, repo, issue_number: issue.number, state: 'closed', state_reason: 'completed',
    });
    core.info(`Closed issue #${issue.number}`);
  }
  return open.map((issue) => issue.number);
}

module.exports = { findStandingIssues, ensureLabel, upsertStandingIssue, closeStandingIssues };
