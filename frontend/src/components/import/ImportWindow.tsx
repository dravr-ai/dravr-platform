// ABOUTME: The /import window: the sources an athlete can import their own history from, and each import's state
// ABOUTME: Opens on the source list (W1); a source opens its import (W2) in place, and #strava or #garmin opens it directly
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { GARMIN_ARCHIVE_SOURCE, STRAVA_ARCHIVE_SOURCE } from '@pierre/shared-types';
import { importSourceCopy, offeredImportSources } from '@pierre/ui-logic';
import { importsApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { DravrLogo } from '../DravrLogo';
import SourceImport from './SourceImport';
import { importStateLine } from './importCopy';

/** The hash a link into the window names each source's import by. */
const SOURCE_HASHES: Record<string, string> = {
  [STRAVA_ARCHIVE_SOURCE]: '#strava',
  [GARMIN_ARCHIVE_SOURCE]: '#garmin',
};

/** The source a link into the window opens: `#strava` the Strava import, `#garmin` the Garmin one; else none, the list. */
function sourceFromHash(): string | null {
  const hash = window.location.hash;
  return Object.keys(SOURCE_HASHES).find((source) => SOURCE_HASHES[source] === hash) ?? null;
}

interface ImportWindowProps {
  /** Leave the window: close it when it was opened as one, else go back to the app. */
  onClose: () => void;
}

export default function ImportWindow({ onClose }: ImportWindowProps) {
  const { t, i18n } = useTranslation();
  const [openSource, setOpenSource] = useState<string | null>(sourceFromHash);

  useEffect(() => {
    const onHash = () => setOpenSource(sourceFromHash());
    window.addEventListener('hashchange', onHash);
    return () => window.removeEventListener('hashchange', onHash);
  }, []);

  const open = (next: string | null) => {
    window.history.pushState(null, '', (next && SOURCE_HASHES[next]) ?? window.location.pathname);
    setOpenSource(next);
  };

  const { data: overview, isLoading, isError } = useQuery({
    queryKey: QUERY_KEYS.imports.overview(),
    queryFn: () => importsApi.getImports(),
  });
  const sources = offeredImportSources(overview?.sources ?? []);
  const opened = openSource ? importSourceCopy(openSource) : undefined;

  return (
    <div className="min-h-dvh bg-surface pad-safe-top">
      <div className="mx-auto w-full max-w-xl px-4 py-6">
        <header className="mb-6 flex items-center justify-between gap-3">
          <div className="flex items-center gap-3">
            <DravrLogo className="h-7 w-7" />
            <h1 className="text-xl font-semibold text-on-surface">{t('providers.import.title')}</h1>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="text-sm font-medium text-primary hover:text-primary-hover"
            data-testid="import-close"
          >
            {t('providers.import.close')}
          </button>
        </header>

        {opened ? (
          <div>
            <button
              type="button"
              onClick={() => open(null)}
              className="mb-4 text-sm font-medium text-primary hover:text-primary-hover"
            >
              <span aria-hidden="true">← </span>
              {t('providers.import.back')}
            </button>
            <SourceImport key={opened.source} copy={opened} enabled={overview?.enabled ?? true} />
          </div>
        ) : (
          <section aria-labelledby="import-sources">
            <p id="import-sources" className="mb-4 text-sm text-on-surface-variant">
              {t('providers.import.intro')}
            </p>
            {isLoading && <p className="text-sm text-on-surface-variant">{t('providers.import.loading')}</p>}
            {isError && <p className="text-sm text-error">{t('providers.import.strava.genericError')}</p>}
            {overview && !overview.enabled && (
              <p className="text-sm text-on-surface-variant" data-testid="import-unavailable">
                {t('providers.import.unavailable')}
              </p>
            )}
            {overview?.enabled && (
              <ul>
                {sources.map((copy, index) => {
                  const latest = overview.imports.find((entry) => entry.source === copy.source);
                  return (
                    <li key={copy.source}>
                      <button
                        type="button"
                        onClick={() => open(copy.source)}
                        data-testid={`import-source-${copy.source}`}
                        className={`group flex w-full items-center gap-3 border-b ghost-border-faint py-4 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-primary/50 ${
                          index === 0 ? 'border-t' : ''
                        }`}
                      >
                        <span className="flex min-w-0 flex-1 flex-col">
                          <span className="text-sm font-medium text-on-surface">{t(copy.name)}</span>
                          <span className="text-xs text-on-surface-variant">{t(copy.hint)}</span>
                          {latest && (
                            <span className="mt-1 text-xs text-on-surface" data-testid={`import-state-${copy.source}`}>
                              {importStateLine(t, latest, copy, i18n.language)}
                            </span>
                          )}
                        </span>
                        <span aria-hidden="true" className="text-on-surface-variant">
                          ›
                        </span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
          </section>
        )}
      </div>
    </div>
  );
}
