// ABOUTME: The /import window: the sources an athlete can import their own history from, and each import's state
// ABOUTME: Opens on the source list (W1); a source opens its import (W2) in place, and #strava opens it directly
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { STRAVA_ARCHIVE_SOURCE, type ArchiveImport } from '@pierre/shared-types';
import { importsApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { DravrLogo } from '../DravrLogo';
import StravaImport from './StravaImport';
import { importStateLine } from './importCopy';

/** Which screen of the window is showing. */
type View = 'sources' | 'strava';

/** The view a link into the window asks for: `#strava` opens the Strava import. */
function viewFromHash(): View {
  return window.location.hash === '#strava' ? 'strava' : 'sources';
}

interface ImportWindowProps {
  /** Leave the window: close it when it was opened as one, else go back to the app. */
  onClose: () => void;
}

export default function ImportWindow({ onClose }: ImportWindowProps) {
  const { t, i18n } = useTranslation();
  const [view, setView] = useState<View>(viewFromHash);

  useEffect(() => {
    const onHash = () => setView(viewFromHash());
    window.addEventListener('hashchange', onHash);
    return () => window.removeEventListener('hashchange', onHash);
  }, []);

  const open = (next: View) => {
    window.history.pushState(null, '', next === 'strava' ? '#strava' : window.location.pathname);
    setView(next);
  };

  const { data: overview, isLoading, isError } = useQuery({
    queryKey: QUERY_KEYS.imports.overview(),
    queryFn: () => importsApi.getImports(),
  });
  const stravaImport: ArchiveImport | undefined = overview?.imports.find(
    (entry) => entry.source === STRAVA_ARCHIVE_SOURCE,
  );

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

        {view === 'strava' ? (
          <div>
            <button
              type="button"
              onClick={() => open('sources')}
              className="mb-4 text-sm font-medium text-primary hover:text-primary-hover"
            >
              <span aria-hidden="true">← </span>
              {t('providers.import.back')}
            </button>
            <StravaImport enabled={overview?.enabled ?? true} />
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
              <button
                type="button"
                onClick={() => open('strava')}
                data-testid="import-source-strava"
                className="group flex w-full items-center gap-3 border-t border-b ghost-border-faint py-4 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-primary/50"
              >
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="text-sm font-medium text-on-surface">{t('providers.import.sources.strava')}</span>
                  <span className="text-xs text-on-surface-variant">{t('providers.import.sources.stravaHint')}</span>
                  {stravaImport && (
                    <span className="mt-1 text-xs text-on-surface" data-testid="import-state-strava">
                      {importStateLine(t, stravaImport, i18n.language)}
                    </span>
                  )}
                </span>
                <span aria-hidden="true" className="text-on-surface-variant">
                  ›
                </span>
              </button>
            )}
          </section>
        )}
      </div>
    </div>
  );
}
