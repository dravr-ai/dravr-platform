// ABOUTME: Data export on the web privacy pane — the athlete downloads their own data as one JSON file (Law 25 s.27)
// ABOUTME: Fetches through dataExportApi, saves the file, and says plainly when the hour's exports are used or the file is partial

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useMutation } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import {
  apiErrorStatus,
  EXPORT_CUT_SHORT,
  EXPORT_TAIL_BYTES,
  exportPartialKey,
  readExportTail,
} from '@pierre/ui-logic';
import { dataExportApi, type UserDataExportFile } from '../services/api';
import { Button, Section } from './ui';

/** The last bytes of the file as text, read without loading the rest. */
function readTail(content: Blob): Promise<string> {
  const tail = content.slice(Math.max(0, content.size - EXPORT_TAIL_BYTES));
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsText(tail);
  });
}

/**
 * Download the export, and the sentence it is saved with when the server
 * stopped it at a limit. A file that does not end like a document was cut
 * short in transit, and is not saved.
 */
async function downloadExport(): Promise<{ file: UserDataExportFile; partialKey: string | null }> {
  const file = await dataExportApi.exportMyData();
  const tail = readExportTail(await readTail(file.content));
  if (!tail) {
    throw new Error(EXPORT_CUT_SHORT);
  }
  return { file, partialKey: exportPartialKey(tail) };
}

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
  const [partial, setPartial] = useState<string | null>(null);

  const mutation = useMutation({
    mutationFn: downloadExport,
    onMutate: () => {
      setFailure(null);
      setPartial(null);
    },
    // A partial file is still the athlete's data: it is saved, and the
    // section says what it lacks.
    onSuccess: ({ file, partialKey }) => {
      saveAsFile(file);
      setPartial(partialKey && t(partialKey));
    },
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
      {partial && (
        <p className="mt-3 text-sm text-on-surface" role="status" data-testid="data-export-partial">
          {partial}
        </p>
      )}
      {failure && (
        <p className="mt-3 text-sm text-error" role="alert" data-testid="data-export-failure">
          {failure}
        </p>
      )}
    </Section>
  );
}
