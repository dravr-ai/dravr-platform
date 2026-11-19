// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Settings › Physiology — the athlete's anchors per sport family, each with its value, range, source, date and confidence
// ABOUTME: The athlete changes one or adds one here, saved through the one physiology writer as their own (carnet#1516, carnet#1561)

import { useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import { PHYSIOLOGY_EDIT_KEY, PHYSIOLOGY_KEY } from '@pierre/shared-constants';
import { describeApiError, type AnchorFormTarget } from '@pierre/ui-logic';
import { Button, EmptyState, Section } from '../ui';
import { usePhysiology, usePhysiologyEdit } from '../../hooks/usePhysiology';
import PhysiologyAnchorEditor from './PhysiologyAnchorEditor';
import PhysiologyAnchorList from './PhysiologyAnchorList';

export default function PhysiologySettings() {
  const { t } = useTranslation();
  const physiology = usePhysiology();
  const save = usePhysiologyEdit();
  const [target, setTarget] = useState<AnchorFormTarget | null>(null);

  const renderBody = () => {
    if (physiology.isPending) {
      return <div className="pierre-spinner h-5 w-5" role="status" aria-label={t('common.loading')} />;
    }
    if (physiology.isError) {
      return (
        <p role="alert" className="text-sm text-error" data-testid="physiology-load-failed">
          {describeApiError(physiology.error, { t, fallbackKey: PHYSIOLOGY_KEY.loadFailed })}
        </p>
      );
    }
    return (
      <div className="space-y-6">
        {physiology.data.sports.length === 0 ? (
          <EmptyState data-testid="physiology-empty">{t(PHYSIOLOGY_KEY.empty)}</EmptyState>
        ) : (
          <PhysiologyAnchorList
            sports={physiology.data.sports}
            onEdit={(sport, anchor) => setTarget({ mode: 'edit', sport, anchor })}
          />
        )}
        <Button variant="outline" onClick={() => setTarget({ mode: 'add' })} data-testid="physiology-add">
          {t(PHYSIOLOGY_EDIT_KEY.add)}
        </Button>
      </div>
    );
  };

  return (
    <div className="space-y-10" data-testid="physiology-settings">
      <Section title={t(PHYSIOLOGY_KEY.title)} description={t(PHYSIOLOGY_KEY.blurb)}>
        {renderBody()}
      </Section>
      <PhysiologyAnchorEditor
        target={target}
        offered={physiology.data?.offered ?? []}
        onClose={() => setTarget(null)}
        onSave={(anchorSave) => save.mutateAsync(anchorSave)}
      />
    </div>
  );
}
