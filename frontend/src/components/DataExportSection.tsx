// ABOUTME: Data export on the web privacy pane — the athlete downloads their own data as one JSON file (Law 25 s.27)
// ABOUTME: Fetches through dataExportApi, hands the browser a file to save, and says plainly when the hour's exports are used

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useMutation } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { apiErrorStatus } from '@pierre/ui-logic';
import { dataExportApi } from '../services/api';
import { Button, Section } from './ui';

/** Hand the document to the browser as a downloaded JSON file. */
function saveAsFile({ fileName, content }: { fileName: string; content: Blob }): void {
  const url = URL.createObjectURL(new Blob([content], { type: 'application/json' }));
  const link = document.createElement('a');
  link.href = url;
  link.download = fileName;
  document.body.appendChild(link);
  link.click();
  link.remove();
  URL.revokeObjectURL(url);
}

export default function DataExportSection() {
  const { t } = useTranslation();
  const [failure, setFailure] = useState<string | null>(null);

  const mutation = useMutation({
    mutationFn: () => dataExportApi.exportMyData(),
    onMutate: () => setFailure(null),
    onSuccess: saveAsFile,
    onError: (err: unknown) => {
      const status = apiErrorStatus(err);
      const limited = status === 429 || status === 503;
      setFailure(t(limited ? 'dataExport.rateLimited' : 'dataExport.failed'));
    },
  });

  return (
    <Section
      title={t('dataExport.title')}
      description={t('dataExport.description')}
      data-testid="data-export-section"
    >
      <Button
        variant="secondary"
        size="sm"
        onClick={() => mutation.mutate()}
        loading={mutation.isPending}
        data-testid="data-export-action"
      >
        {mutation.isPending ? t('dataExport.preparing') : t('dataExport.action')}
      </Button>
      {failure && (
        <p className="mt-3 text-sm text-error" role="alert" data-testid="data-export-failure">
          {failure}
        </p>
      )}
    </Section>
  );
}
