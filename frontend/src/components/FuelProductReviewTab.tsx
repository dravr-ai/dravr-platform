// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Super-admin review of the global fuel-product bank — the paged proposal queue, a drawer to correct, approve or reject one, and the paged live catalogue
// ABOUTME: The drawer (fuelReview/) shows every stored photo and the reader's notes; each fact and identity field can be kept, changed or cleared

import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';
import { QUERY_KEYS } from '../constants/queryKeys';
import { adminApi, type FuelProposalStatus } from '../services/api/admin';
import { Badge, Button, Section, Select, Tabs } from './ui';
import Pager from './fuelReview/Pager';
import ProposalDrawer from './fuelReview/ProposalDrawer';
import { PAGE_SIZE, SOURCE_LABELS, STATUS_OPTIONS, formLabel, formatFact, productName } from './fuelReview/fuelReviewModel';

function ProposalQueue({ onOpen }: { onOpen: (id: string) => void }) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<FuelProposalStatus>('pending');
  const [offset, setOffset] = useState(0);
  const proposals = useQuery({
    queryKey: QUERY_KEYS.adminFuelProducts.proposals(status, offset),
    queryFn: () => adminApi.listFuelProposals(status, PAGE_SIZE, offset),
  });
  // Reviewing the last row of the last page empties that page: step back to
  // the page that now holds the queue's tail rather than show an empty one.
  const emptiedPage = offset > 0 && proposals.data !== undefined && proposals.data.proposals.length === 0;
  const tailOffset = Math.max(0, Math.floor(((proposals.data?.total ?? 0) - 1) / PAGE_SIZE) * PAGE_SIZE);
  useEffect(() => {
    if (emptiedPage) setOffset(tailOffset);
  }, [emptiedPage, tailOffset]);

  const renderBody = () => {
    if (proposals.isPending) {
      return <div className="pierre-spinner h-5 w-5" role="status" aria-label={t('common.loading')} />;
    }
    if (proposals.isError) {
      return (
        <p role="alert" className="text-sm text-error">
          {describeApiError(proposals.error, { t, fallbackKey: 'errors.unknown' })}
        </p>
      );
    }
    const page = proposals.data;
    if (page.total === 0) {
      return (
        <p className="text-sm text-on-surface-variant" data-testid="fuel-proposals-empty">
          No {status} proposals.
        </p>
      );
    }
    return (
      <>
        <ul className="divide-y divide-outline-variant" data-testid="fuel-proposals-list">
          {page.proposals.map((proposal) => (
          <li key={proposal.id} className="flex items-center justify-between gap-3 py-3">
            <div className="min-w-0">
              <p className="truncate text-sm font-medium text-on-surface">{productName(proposal)}</p>
              <p className="text-xs text-on-surface-variant">
                {formLabel(proposal.form)} · {SOURCE_LABELS[proposal.source_kind] ?? proposal.source_kind} ·{' '}
                {proposal.facts.carbs_g !== undefined ? `${proposal.facts.carbs_g} g carbs` : 'carbs not stated'} ·{' '}
                {new Date(proposal.created_at).toLocaleDateString()}
              </p>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              {proposal.gate_flags.length > 0 && (
                <Badge variant="warning">
                  {proposal.gate_flags.length} flag{proposal.gate_flags.length === 1 ? '' : 's'}
                </Badge>
              )}
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={() => onOpen(proposal.id)}
                data-testid={`fuel-proposal-open-${proposal.id}`}
              >
                {status === 'pending' ? 'Review' : 'View'}
              </Button>
            </div>
          </li>
          ))}
        </ul>
        <Pager
          offset={offset}
          pageSize={PAGE_SIZE}
          shown={page.proposals.length}
          total={page.total}
          noun={`${status} proposals`}
          onOffset={setOffset}
          testId="fuel-proposals-pager"
        />
      </>
    );
  };

  return (
    <Section
      title="Proposals"
      description="Label facts waiting for an operator. Nothing reaches athletes until it is approved."
      actions={
        <Select
          aria-label="Proposal status"
          size="sm"
          value={status}
          options={STATUS_OPTIONS.map((o) => ({ value: o.value, label: o.label }))}
          onChange={(e) => {
            setStatus(e.target.value as FuelProposalStatus);
            setOffset(0);
          }}
        />
      }
      data-testid="fuel-proposals"
    >
      {renderBody()}
    </Section>
  );
}

function LiveCatalogue() {
  const { t } = useTranslation();
  const [offset, setOffset] = useState(0);
  const catalogue = useQuery({
    queryKey: QUERY_KEYS.adminFuelProducts.catalogue(offset),
    queryFn: () => adminApi.listFuelCatalogue(PAGE_SIZE, offset),
  });

  const renderBody = () => {
    if (catalogue.isPending) {
      return <div className="pierre-spinner h-5 w-5" role="status" aria-label={t('common.loading')} />;
    }
    if (catalogue.isError) {
      return (
        <p role="alert" className="text-sm text-error">
          {describeApiError(catalogue.error, { t, fallbackKey: 'errors.unknown' })}
        </p>
      );
    }
    const page = catalogue.data;
    if (page.total === 0) {
      return (
        <p className="text-sm text-on-surface-variant" data-testid="fuel-catalogue-empty">
          The catalogue is empty: approve a proposal to put a product in front of athletes.
        </p>
      );
    }
    return (
      <>
        <ul className="divide-y divide-outline-variant" data-testid="fuel-catalogue-list">
          {page.products.map((row) => {
          const facts = row.current.facts;
          return (
            <li key={row.product.id} className="py-3">
              <p className="text-sm font-medium text-on-surface">{productName(row.product)}</p>
              <p className="text-xs text-on-surface-variant">
                {formLabel(row.product.form)}
                {row.product.gtin && ` · GTIN ${row.product.gtin}`} · {formatFact(facts.carbs_g, 'g')} carbs per serving
                {facts.glucose_fructose_ratio !== undefined && ` · ratio 1:${facts.glucose_fructose_ratio}`} ·{' '}
                {row.version_count} version{row.version_count === 1 ? '' : 's'}
                {row.current.approved_at && ` · current since ${new Date(row.current.approved_at).toLocaleDateString()}`}
              </p>
            </li>
          );
          })}
        </ul>
        <Pager
          offset={offset}
          pageSize={PAGE_SIZE}
          shown={page.products.length}
          total={page.total}
          noun="products"
          onOffset={setOffset}
          testId="fuel-catalogue-pager"
        />
      </>
    );
  };

  return (
    <Section
      title="Live catalogue"
      description="What athletes can carry today, with each product's current label facts."
      data-testid="fuel-catalogue"
    >
      {renderBody()}
    </Section>
  );
}

export default function FuelProductReviewTab() {
  const [view, setView] = useState<'queue' | 'catalogue'>('queue');
  const [openId, setOpenId] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  return (
    <div className="space-y-6" data-testid="fuel-product-review">
      <Tabs
        variant="underline"
        size="sm"
        activeTab={view}
        onChange={(id) => setView(id === 'catalogue' ? 'catalogue' : 'queue')}
        tabs={[
          { id: 'queue', label: 'Review queue' },
          { id: 'catalogue', label: 'Live catalogue' },
        ]}
      />
      {notice && (
        <p role="status" className="text-sm text-success" data-testid="fuel-review-notice">
          {notice}
        </p>
      )}
      {view === 'queue' ? (
        <ProposalQueue
          onOpen={(id) => {
            setNotice(null);
            setOpenId(id);
          }}
        />
      ) : (
        <LiveCatalogue />
      )}
      {openId && (
        <ProposalDrawer
          key={openId}
          proposalId={openId}
          onClose={() => setOpenId(null)}
          onDone={(message) => {
            setOpenId(null);
            setNotice(message);
          }}
        />
      )}
    </div>
  );
}
