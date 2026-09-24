#!/usr/bin/env bash
# ABOUTME: Pins scripts/ci/standing-issue.cjs, the find-or-create every monitor workflow's standing issue goes through
# ABOUTME: Drives it against a stubbed Octokit whose tracker holds more issues than one page, then checks every caller uses it
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The regression this exists for is quiet: a lookup that reads only the first
# page of `issues.listForRepo` finds its standing issue until enough newer issues
# carry the label, then files a duplicate on every run. The stub serves 250
# issues and puts the standing one on the last page, and its un-paginated
# listForRepo answers with the first page only, so a one-page lookup fails here.
#
# Usage: scripts/ci/standing-issue.test.sh — bash and node, no network.

set -euo pipefail
cd "$(dirname "$0")/../.."
WORK=$(mktemp -d); trap 'rm -rf "${WORK}"' EXIT

echo "standing-issue: module"
node - "$PWD/scripts/ci/standing-issue.cjs" <<'JS'
const standing = require(process.argv[2]);
const failures = [];
const expect = (label, got, want) => {
  const ok = JSON.stringify(got) === JSON.stringify(want);
  console.log(`  ${ok ? 'ok  ' : 'FAIL'} ${label}${ok ? '' : `: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`}`);
  if (!ok) failures.push(label);
};

// A tracker of `filler` issues newest-first, then `extra` (the old ones).
function tracker({ filler = 249, extra = [], labels = ['incident'] } = {}) {
  const issues = [
    ...Array.from({ length: filler }, (_, i) => ({ number: 10000 - i, title: `noise ${i}`, state: 'open' })),
    ...extra,
  ];
  const calls = [];
  const notFound = Object.assign(new Error('Not Found'), { status: 404 });
  const github = {
    rest: {
      issues: {
        listForRepo: async (p) => ({ data: issues.slice(0, p.per_page ?? 30) }),
        getLabel: async (p) => {
          calls.push(['getLabel', p.name]);
          if (!labels.includes(p.name)) throw notFound;
          return { data: {} };
        },
        createLabel: async (p) => { calls.push(['createLabel', p.name, p.color, p.description]); },
        create: async (p) => { calls.push(['create', p.title, p.labels]); return { data: { number: 20000 } }; },
        update: async (p) => { calls.push(['update', p.issue_number, p.title ?? null, p.state ?? null, p.state_reason ?? null]); },
        createComment: async (p) => { calls.push(['comment', p.issue_number, p.body]); },
      },
    },
    paginate: async (fn, p) => {
      if (fn !== github.rest.issues.listForRepo) throw new Error('paginate called with an unexpected method');
      calls.push(['list', p.labels, p.state, p.per_page]);
      return issues.filter((i) => p.state === 'all' || i.state === p.state);
    },
  };
  const core = { info: () => {} };
  return { github, core, calls };
}
const owner = 'dravr-ai';
const repo = 'dravr-platform';

(async () => {
  {
    const t = tracker({ extra: [{ number: 7, title: 'Probe failing', state: 'open' }] });
    const got = await standing.upsertStandingIssue(t.github, t.core, {
      owner, repo, label: 'incident', title: 'Probe failing', body: 'b2', onExisting: 'update',
    });
    expect('a standing issue on page 3 of 3 is found and updated, not duplicated', got, { number: 7, created: false });
    expect('  ...reading every page, open, 100 at a time, by label', t.calls[0], ['list', 'incident', 'open', 100]);
    expect('  ...and replacing its title and body only', t.calls.slice(1), [['update', 7, 'Probe failing', null, null]]);
  }
  {
    const t = tracker({ extra: [{ number: 8, title: 'Probe failing', state: 'open', pull_request: {} }] });
    const got = await standing.upsertStandingIssue(t.github, t.core, {
      owner, repo, label: 'dependencies', title: 'Probe failing', body: 'b', onExisting: 'update',
      newLabel: { color: '0366d6', description: 'Dependency updates' },
    });
    expect('a pull request with the same title is never taken for the issue', got, { number: 20000, created: true });
    expect('  ...the missing label is created with its colour before filing',
      t.calls.slice(1), [['getLabel', 'dependencies'], ['createLabel', 'dependencies', '0366d6', 'Dependency updates'],
        ['create', 'Probe failing', ['dependencies']]]);
  }
  {
    const t = tracker({ labels: ['dravr-platform'] });
    await standing.upsertStandingIssue(t.github, t.core, {
      owner, repo: 'dravr-carnet', label: 'dravr-platform', title: '[platform] x', body: 'b',
      onExisting: 'comment', labels: ['dravr-platform', 'bug', 'critical'],
    });
    expect('with no open issue it files one carrying every label given, and touches no label without newLabel',
      t.calls.slice(1), [['create', '[platform] x', ['dravr-platform', 'bug', 'critical']]]);
  }
  {
    const t = tracker({ extra: [{ number: 9, title: '[platform] x', state: 'open' }] });
    await standing.upsertStandingIssue(t.github, t.core, {
      owner, repo, label: 'dravr-platform', title: '[platform] x', body: 'today', onExisting: 'comment',
    });
    expect("onExisting 'comment' appends the body and leaves the issue as it was", t.calls.slice(1), [['comment', 9, 'today']]);
  }
  {
    const t = tracker({ extra: [{ number: 11, title: 'Copilot CLI update available: 1.2.3', state: 'open' }] });
    await standing.upsertStandingIssue(t.github, t.core, {
      owner, repo, label: 'dependencies', title: 'Copilot pin drift: CLI 1.2.4',
      titlePrefixes: ['Copilot pin drift:', 'Copilot CLI update available:'], body: 'b', onExisting: 'update',
    });
    expect('a prefix lookup finds the issue under its older title and retitles it',
      t.calls.slice(1), [['update', 11, 'Copilot pin drift: CLI 1.2.4', null, null]]);
  }
  {
    const t = tracker({ extra: [
      { number: 12, title: 'Copilot pin drift: SDK 0.3', state: 'open' },
      { number: 13, title: 'Copilot pin drift: CLI 1.0', state: 'open' },
      { number: 14, title: 'Copilot pin drift: CLI 0.9', state: 'closed' },
    ] });
    const closed = await standing.closeStandingIssues(t.github, t.core, {
      owner, repo, label: 'dependencies', titlePrefixes: ['Copilot pin drift:'], comment: 'current',
    });
    expect('close comments on and closes every open match, and only those', closed, [12, 13]);
    expect('  ...as completed', t.calls.slice(1), [
      ['comment', 12, 'current'], ['update', 12, null, 'closed', 'completed'],
      ['comment', 13, 'current'], ['update', 13, null, 'closed', 'completed']]);
  }
  {
    const t = tracker({ extra: [{ number: 15, title: '[platform] lane: A', state: 'closed' }] });
    const all = await standing.findStandingIssues(t.github, {
      owner, repo, label: 'dravr-platform', state: 'all', titlePrefixes: ['[platform] lane: '],
    });
    expect('state all returns closed matches too', all.map((i) => i.number), [15]);
  }
  {
    const t = tracker();
    const denied = Object.assign(new Error('Forbidden'), { status: 403 });
    t.github.rest.issues.getLabel = async () => { throw denied; };
    let thrown = null;
    try {
      await standing.ensureLabel(t.github, { owner, repo, name: 'incident', color: 'b60205', description: 'x' });
    } catch (e) { thrown = e.status; }
    expect('a label read that fails for any reason but 404 is rethrown, never answered with a create',
      [thrown, t.calls.filter((c) => c[0] === 'createLabel').length], [403, 0]);
  }
  {
    const t = tracker();
    let thrown = null;
    try {
      await standing.findStandingIssues(t.github, { owner, repo, label: 'x', title: 'a', titlePrefixes: ['a'] });
    } catch (e) { thrown = e.message; }
    expect('a lookup given both a title and prefixes is refused', typeof thrown, 'string');
  }
  if (failures.length) { console.error(`FAIL: ${failures.length} case(s)`); process.exit(1); }
})().catch((e) => { console.error(`FAIL: ${e.stack}`); process.exit(1); });
JS

echo "standing-issue: callers"
# Every github-script step that looks a standing issue up goes through the module:
# a hand-rolled listForRepo anywhere in a workflow is the second lookup this file
# exists to prevent, and a workflow that requires the module must check it out.
# Each requiring script is also compiled the way github-script compiles it, so a
# stray brace fails here rather than on the day the monitor first alarms.
python3 - "${WORK}" <<'PY'
import glob, os, sys, yaml
problems, callers = [], []
for path in sorted(glob.glob(".github/workflows/*.yml")):
    for job_id, job in (yaml.safe_load(open(path)).get("jobs") or {}).items():
        steps = job.get("steps") or []
        checked_out = False
        for step in steps:
            if str(step.get("uses", "")).startswith("actions/checkout"):
                sparse = str((step.get("with") or {}).get("sparse-checkout", ""))
                checked_out = checked_out or not sparse or "scripts/ci/standing-issue.cjs" in sparse
            script = str((step.get("with") or {}).get("script", ""))
            if not str(step.get("uses", "")).startswith("actions/github-script"):
                continue
            if "listForRepo" in script:
                problems.append(f"{path} job {job_id}: hand-rolled issues.listForRepo lookup; use scripts/ci/standing-issue.cjs")
            if "standing-issue.cjs" in script:
                callers.append(f"{path.split('/')[-1]}:{job_id}")
                name = f"{os.path.basename(path)}.{job_id}.{len(callers)}.js"
                open(os.path.join(sys.argv[1], name), "w").write(script)
                if not checked_out:
                    problems.append(f"{path} job {job_id}: requires standing-issue.cjs before any checkout that holds it")
for p in problems:
    print(f"  FAIL {p}")
if not callers:
    problems.append("no workflow requires standing-issue.cjs — the scan found nothing to check")
    print("  FAIL no workflow requires standing-issue.cjs — the scan found nothing to check")
if problems:
    sys.exit(1)
print(f"  ok   {len(callers)} standing-issue step(s) in {len(set(callers))} job(s), every lookup through the module: {' '.join(sorted(set(callers)))}")
PY
node - "${WORK}" <<'JS'
const fs = require('fs');
const path = require('path');
const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor;
const bad = [];
const files = fs.readdirSync(process.argv[2]).filter((f) => f.endsWith('.js'));
for (const f of files) {
  try {
    new AsyncFunction('require', 'github', 'context', 'core', 'exec', 'glob', 'io', fs.readFileSync(path.join(process.argv[2], f), 'utf8'));
  } catch (e) {
    bad.push(`${f}: ${e.message}`);
  }
}
if (bad.length) { console.error(`  FAIL a standing-issue script does not compile:\n    ${bad.join('\n    ')}`); process.exit(1); }
console.log(`  ok   all ${files.length} of them compile`);
JS

echo
echo "all standing-issue scenarios pass"
