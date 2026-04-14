// ABOUTME: Phase B Sprint C16 — admin eval harness tab browsing pierre-evals golden fixtures
// ABOUTME: Read-only per-fixture and per-case view. Live runs are a later sprint.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import {
  adminApi,
  type EvalFixtureBrowserResponse,
  type EvalFixtureSummary,
} from '../services/api/admin';
import { Card, Button, Badge } from './ui';

function totalAssertions(fixture: EvalFixtureSummary): number {
  return fixture.cases.reduce(
    (acc, c) => acc + c.must_contain_total + c.must_not_contain_total,
    0,
  );
}

export default function EvalHarnessTab() {
  const [expandedFixture, setExpandedFixture] = useState<string | null>(null);

  const { data, isLoading, isError, error, refetch, isFetching } =
    useQuery<EvalFixtureBrowserResponse>({
      queryKey: ['admin', 'evals', 'fixtures'] as const,
      queryFn: () => adminApi.getEvalFixtureBrowser(),
    });

  const personaCounts = useMemo(() => {
    if (!data) return new Map<string, number>();
    const counts = new Map<string, number>();
    for (const fixture of data.fixtures) {
      for (const c of fixture.cases) {
        counts.set(c.persona, (counts.get(c.persona) ?? 0) + 1);
      }
    }
    return counts;
  }, [data]);

  return (
    <div className="space-y-4">
      <Card className="p-6">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="text-xl font-semibold text-gray-900 dark:text-white">
              Eval harness — golden fixtures
            </h2>
            <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
              Read-only browser over the <code className="rounded bg-gray-100 px-1 dark:bg-gray-800">pierre-evals</code>{' '}
              golden fixture set used by the Tier 5 evaluation harness.
              Live evaluation runs land in a follow-up sprint; for now
              this tab shows which scenarios ship with the release and
              what each case asserts.
            </p>
          </div>
          <Button onClick={() => refetch()} variant="secondary" disabled={isFetching}>
            {isFetching ? 'Refreshing…' : 'Refresh'}
          </Button>
        </div>

        <div className="mt-6 grid grid-cols-2 gap-3 sm:grid-cols-4">
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Fixture files
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-white">
              {data?.fixture_count ?? 0}
            </div>
          </div>
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Total cases
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-white">
              {data?.case_total ?? 0}
            </div>
          </div>
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Distinct personas
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-white">
              {personaCounts.size}
            </div>
          </div>
          <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 dark:border-gray-700 dark:bg-gray-800">
            <div className="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
              Assertions
            </div>
            <div className="mt-1 text-2xl font-semibold text-gray-900 dark:text-white">
              {data?.fixtures.reduce((acc, f) => acc + totalAssertions(f), 0) ?? 0}
            </div>
          </div>
        </div>

        {data && (
          <p className="mt-4 text-xs text-gray-400 dark:text-gray-500">
            Scanned directory:{' '}
            <code className="rounded bg-gray-100 px-1 dark:bg-gray-800">
              {data.scanned_dir}
            </code>
          </p>
        )}
      </Card>

      {isLoading ? (
        <Card className="p-12">
          <div className="flex justify-center">
            <div className="pierre-spinner" />
          </div>
        </Card>
      ) : isError ? (
        <Card className="p-6">
          <p className="text-sm text-red-600 dark:text-red-400">
            Failed to load eval fixtures:{' '}
            {error instanceof Error ? error.message : String(error)}
          </p>
          <p className="mt-2 text-xs text-gray-500 dark:text-gray-400">
            Make sure the server was started with <code>tools-verification</code>{' '}
            enabled and <code>PIERRE_EVALS_FIXTURES_DIR</code> points at a
            valid directory.
          </p>
        </Card>
      ) : !data || data.fixtures.length === 0 ? (
        <Card className="p-12 text-center">
          <p className="text-gray-500 dark:text-gray-400">
            No fixture files found in the scanned directory.
          </p>
          <p className="mt-1 text-xs text-gray-400 dark:text-gray-500">
            Drop a <code>.jsonl</code> file into{' '}
            <code className="rounded bg-gray-100 px-1 dark:bg-gray-800">
              crates/pierre-evals/fixtures/
            </code>{' '}
            or point <code>PIERRE_EVALS_FIXTURES_DIR</code> elsewhere.
          </p>
        </Card>
      ) : (
        data.fixtures.map((fixture) => {
          const isExpanded = expandedFixture === fixture.name;
          return (
            <Card key={fixture.name} className="overflow-hidden">
              <button
                type="button"
                onClick={() =>
                  setExpandedFixture(isExpanded ? null : fixture.name)
                }
                className="flex w-full items-center justify-between gap-4 border-b border-gray-200 px-6 py-3 text-left hover:bg-gray-50 dark:border-gray-700 dark:hover:bg-gray-800"
                aria-expanded={isExpanded}
                aria-label={`Toggle ${fixture.name} fixture`}
              >
                <div>
                  <h3 className="text-sm font-semibold text-gray-900 dark:text-white">
                    {fixture.name}
                  </h3>
                  <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
                    {fixture.case_count} case
                    {fixture.case_count === 1 ? '' : 's'} ·{' '}
                    {totalAssertions(fixture)} assertion
                    {totalAssertions(fixture) === 1 ? '' : 's'}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  {fixture.personas.map((p) => (
                    <Badge key={p} variant="info">
                      {p}
                    </Badge>
                  ))}
                </div>
              </button>
              {isExpanded && (
                <div className="divide-y divide-gray-200 dark:divide-gray-700">
                  {fixture.cases.map((c) => (
                    <div
                      key={c.id}
                      className="flex items-center justify-between gap-4 px-6 py-3"
                    >
                      <div>
                        <p className="text-sm font-medium text-gray-900 dark:text-gray-100">
                          {c.label}
                        </p>
                        <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
                          <span className="font-mono">{c.id}</span> · persona{' '}
                          <span className="font-mono">{c.persona}</span>
                        </p>
                      </div>
                      <div className="flex items-center gap-3 text-right text-xs">
                        <div>
                          <div className="text-gray-500 dark:text-gray-400">Turns</div>
                          <div className="font-semibold text-gray-900 dark:text-white">
                            {c.turn_count}
                          </div>
                        </div>
                        <div>
                          <div className="text-gray-500 dark:text-gray-400">Must</div>
                          <div className="font-semibold text-green-600 dark:text-green-400">
                            {c.must_contain_total}
                          </div>
                        </div>
                        <div>
                          <div className="text-gray-500 dark:text-gray-400">Must not</div>
                          <div className="font-semibold text-red-600 dark:text-red-400">
                            {c.must_not_contain_total}
                          </div>
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </Card>
          );
        })
      )}
    </div>
  );
}
