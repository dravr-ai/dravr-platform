// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Store group of the agent edit sheet — submit one of the athlete's own agents to the admin review queue
// ABOUTME: One action; the confirmation or the server's refusal is said in place, under it

import { useMutation } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';
import { coachesApi } from '../../services/api';
import { Button, Section } from '../ui';

export interface CoachStoreSubmitProps {
  agentId: string;
}

export default function CoachStoreSubmit({ agentId }: CoachStoreSubmitProps) {
  const { t } = useTranslation();
  const submit = useMutation({ mutationFn: () => coachesApi.submitToStore(agentId) });

  return (
    <Section
      title={t('discover.submitToStore')}
      description={t('discover.submitToStoreHint')}
      headingLevel={3}
      data-testid="agent-store-submit"
    >
      {submit.isSuccess ? (
        <p role="status" className="text-sm text-on-surface-variant" data-testid="agent-store-submitted">
          {t('discover.submittedToStore')}
        </p>
      ) : (
        <Button
          type="button"
          variant="secondary"
          size="sm"
          disabled={submit.isPending}
          onClick={() => submit.mutate()}
          data-testid="agent-store-submit-button"
        >
          {t('discover.submitToStore')}
        </Button>
      )}
      {submit.isError && (
        <p role="alert" className="mt-2 text-sm text-error">
          {describeApiError(submit.error, { t, fallbackKey: 'discover.submitToStoreFailed' })}
        </p>
      )}
    </Section>
  );
}
