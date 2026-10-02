// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One activity's own view, opened from Home — its map, then a chat about it, then its figures, splits and laps, each table scrolling inside itself
// ABOUTME: The chat is the chat surface itself, embedded; its suggested questions name the activity so the agent reads the right one

import { useCallback, useState, type FormEvent } from 'react';
import { ArrowLeft, Send } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import {
  ACTIVITY_PROMPTS,
  activityFigures,
  activityFirstLine,
  lapsTable,
  splitsTable,
  type SegmentTable,
} from '@pierre/ui-logic';
import type { ActivityDetailResponse } from '@pierre/shared-types';
import ChatTab, { type PendingComposerAction } from '../ChatTab';
import { Button, IconButton, Input } from '../ui';
import { EmptyState } from '../ui/EmptyState';
import { Section } from '../ui/Section';
import { ActivityMap } from '../home/ActivityMap';
import { DRAFT_DATE, ROW_DATE, formatInstant, sportLabel } from '../home/homeFormat';
import { useActivityConversation } from '../../hooks/useActivityConversation';
import { useActivityDetail } from '../../hooks/useActivityDetail';
import type { ActivityRef } from './activityRoute';

interface ActivityViewProps {
  /** The activity the Home row or the deep link named. */
  activity: ActivityRef;
  /** Leave for Home — the browser's Back does the same. */
  onBack: () => void;
  /** Dashboard route navigator, `tab[/subview]`. */
  onNavigate: (route: string) => void;
}

/** The header row: back to Home, then the activity's name. */
function ViewHeader({ title, onBack }: { title: string; onBack: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="flex h-[52px] flex-shrink-0 items-center gap-2 border-b ghost-border px-3 md:px-5">
      <IconButton aria-label={t('common.back')} title={t('common.back')} onClick={onBack} data-testid="activity-back">
        <ArrowLeft className="h-4 w-4" aria-hidden="true" />
      </IconButton>
      <h2 className="min-w-0 truncate font-display text-xl font-semibold text-on-surface" data-testid="activity-title">
        {title}
      </h2>
    </div>
  );
}

/** The activity's figures, label over value, as many to a row as the width holds; two in the side panel. */
function Figures({ detail }: { detail: ActivityDetailResponse }) {
  const { t, language } = useTranslation();
  return (
    <dl className="grid grid-cols-2 gap-x-6 gap-y-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-2" data-testid="activity-figures">
      {activityFigures(t, detail, language).map((figure) => (
        <div key={figure.id} data-testid={`activity-figure-${figure.id}`}>
          <dt className="text-xs text-on-surface-variant">{t(figure.labelKey)}</dt>
          <dd className="mt-0.5 font-mono text-base font-medium text-on-surface">{figure.value}</dd>
        </div>
      ))}
    </dl>
  );
}

/**
 * A splits or laps table; a column no row fills is not drawn.
 *
 * The rows scroll inside the table's own frame, under a header that stays
 * put: a marathon is forty-two of them, and laid out in full they pushed the
 * question field a screen and a half down the page. Below `lg` the frame is
 * capped at about six rows; from `lg` up it takes the height the side panel
 * has left, and never less than about five rows. The frame is a named, focusable region so the keyboard can
 * scroll it.
 */
function Segments({ title, table, testId }: { title: string; table: SegmentTable; testId: string }) {
  const { t } = useTranslation();
  const cell = 'px-2 py-1.5 text-right font-mono lg:whitespace-nowrap';
  const head = 'sticky top-0 bg-surface px-2 py-1.5 text-right text-xs font-medium text-on-surface-variant';
  return (
    <Section
      title={title}
      headingLevel={3}
      data-testid={testId}
      className="lg:flex lg:min-h-52 lg:flex-1 lg:flex-col"
      bodyClassName="lg:flex lg:min-h-0 lg:flex-1 lg:flex-col"
    >
      <div
        role="region"
        aria-label={title}
        tabIndex={0}
        data-testid={`${testId}-scroll`}
        className="max-h-64 overflow-auto focus-ring lg:max-h-none lg:min-h-0 lg:flex-1"
      >
        <table className="w-full text-sm text-on-surface">
          <thead>
            <tr>
              <th scope="col" className={head}>#</th>
              <th scope="col" className={head}>{t('home.activity.figure.distance')}</th>
              <th scope="col" className={head}>{t('home.activity.column.time')}</th>
              {table.hasSpeed && <th scope="col" className={head}>{t(table.speedLabelKey)}</th>}
              {table.hasHeartRate && <th scope="col" className={head}>{t('home.activity.column.heartRate')}</th>}
              {table.hasElevation && <th scope="col" className={head}>{t('home.activity.column.elevation')}</th>}
            </tr>
          </thead>
          <tbody>
            {table.rows.map((row) => (
              <tr key={row.index} className="border-t ghost-border-faint">
                <td className={cell}>{row.index}</td>
                <td className={cell}>{row.distance}</td>
                <td className={cell}>{row.time}</td>
                {table.hasSpeed && <td className={cell}>{row.speed ?? ''}</td>}
                {table.hasHeartRate && <td className={cell}>{row.heartRate ?? ''}</td>}
                {table.hasElevation && <td className={cell}>{row.elevation ?? ''}</td>}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Section>
  );
}

/** The first question an athlete types, before the thread exists. */
function AskField({ onAsk }: { onAsk: (question: string) => void }) {
  const { t } = useTranslation();
  const [question, setQuestion] = useState('');
  const submit = (event: FormEvent) => {
    event.preventDefault();
    const typed = question.trim();
    if (typed === '') return;
    setQuestion('');
    onAsk(typed);
  };
  return (
    <form onSubmit={submit} className="flex items-end gap-2" data-testid="activity-ask">
      <Input
        value={question}
        onChange={(event) => setQuestion(event.target.value)}
        placeholder={t('home.activity.askPlaceholder')}
        aria-label={t('home.activity.askPlaceholder')}
      />
      <Button type="submit" variant="primary" disabled={question.trim() === ''} aria-label={t('chat.sendMessageAria')}>
        <Send className="h-4 w-4" aria-hidden="true" />
      </Button>
    </form>
  );
}

/**
 * The chat about the activity: the suggested questions, then the thread.
 *
 * Every question goes out through the chat surface's own `send`, which opens
 * the thread when there is none, so the reply arrives right here, and the
 * thread is one of the athlete's conversations like any other. The thread is
 * linked to the activity on the server: leaving the view and coming back,
 * reloading, or opening the activity on another device reopens it, and a
 * later question goes to it rather than starting a second conversation about
 * the same workout. A suggested question names the activity by its title and
 * date; a typed first question goes out in the sentence that names it, since
 * the new thread has no other word on which activity it is about.
 */
function ActivityChat({
  activity,
  detail,
  onNavigate,
}: {
  /** The activity as the view was opened on it: the thread is linked to this. */
  activity: ActivityRef;
  detail: ActivityDetailResponse;
  onNavigate: (route: string) => void;
}) {
  const { t, language } = useTranslation();
  const { conversationId, setConversationId } = useActivityConversation(activity.provider, activity.id, detail);
  const [action, setAction] = useState<PendingComposerAction | null>(null);
  const naming = {
    name: detail.activity.name.trim() || sportLabel(t, detail.activity.sport_type),
    date: formatInstant(detail.activity.start_date, language, DRAFT_DATE),
  };
  const send = (text: string) => setAction({ kind: 'send', text });
  const consumed = useCallback(() => setAction(null), []);

  return (
    <Section title={t('home.activity.chatHeading')} headingLevel={3} data-testid="activity-chat">
      <div className="mb-4 flex flex-wrap gap-2" data-testid="activity-prompts">
        {ACTIVITY_PROMPTS.map((prompt) => (
          <button
            key={prompt.id}
            type="button"
            data-testid={`activity-prompt-${prompt.id}`}
            onClick={() => send(t(prompt.textKey, naming))}
            className="rounded-full border ghost-border bg-surface-container-lowest px-3 py-1.5 text-sm text-on-surface transition-colors hover:bg-surface-container-low focus-ring touch-target"
          >
            {t(prompt.labelKey)}
          </button>
        ))}
      </div>
      {/* On a phone the thread grows with its turns inside the page's own
          scroll: a second scroller boxed in a card would cut its rows — the
          day pill first — at the card's border. From md up the card holds a
          thread of its own height beside the figures. */}
      <div
        data-testid="activity-chat-frame"
        className={
          conversationId
            ? 'overflow-hidden rounded-[10px] border ghost-border md:h-[70vh] md:min-h-[420px]'
            : ''
        }
      >
        <ChatTab
          layout="embedded"
          selectedConversation={conversationId}
          onSelectConversation={setConversationId}
          onNavigate={onNavigate}
          pendingComposerAction={action}
          onPendingComposerActionConsumed={consumed}
          embeddedEmptyState={
            <AskField onAsk={(question) => send(activityFirstLine(t, naming, question))} />
          }
        />
      </div>
    </Section>
  );
}

export default function ActivityView({ activity, onBack, onNavigate }: ActivityViewProps) {
  const { t, language } = useTranslation();
  const detail = useActivityDetail(activity.provider, activity.id);

  if (detail.data === undefined) {
    let body;
    if (detail.notFound) {
      body = <EmptyState data-testid="activity-not-found">{t('home.activity.notFound')}</EmptyState>;
    } else if (detail.failed) {
      body = (
        <EmptyState data-testid="activity-failed" action={{ label: t('common.retry'), onClick: detail.refetch }}>
          {t('home.activity.loadFailed')}
        </EmptyState>
      );
    } else {
      body = (
        <div className="flex py-3" role="status" aria-label={t('common.loading')}>
          <div className="pierre-spinner" />
        </div>
      );
    }
    return (
      <div className="flex h-full flex-col" data-testid="activity-view">
        <ViewHeader title={t('app.activity')} onBack={onBack} />
        <div className="mx-auto w-full max-w-[720px] px-4 py-6 md:px-6">{body}</div>
      </div>
    );
  }

  const data = detail.data;
  const sport = sportLabel(t, data.activity.sport_type);
  const splits = splitsTable(data, language);
  const laps = lapsTable(data, language);
  // The chat comes straight after the map, in the document and on screen, so
  // the question field is there on arrival at every width; the figures,
  // splits and laps read after it. Below `lg` they follow it down the one
  // column. From `lg` up they leave the page's scroll for a panel pinned to
  // the view's right edge, beside the map — still after the chat for the
  // keyboard and a screen reader, as a right-hand column is. The panel is
  // positioned against the body under the header rather than the scroller,
  // which is why the scroller and the column carry no `relative`: an ancestor
  // outside the scroller does not move with it, and the panel starts where
  // the header ends whatever the header's height. On a short window each
  // table keeps a few rows' height and the panel scrolls as a whole.
  return (
    <div className="flex h-full flex-col" data-testid="activity-view">
      <ViewHeader title={data.activity.name.trim() || sport} onBack={onBack} />
      <div className="relative min-h-0 flex-1" data-testid="activity-body">
        <div className="h-full overflow-y-auto lg:pr-[420px]" data-testid="activity-scroll">
          <div className="mx-auto w-full max-w-[720px] space-y-8 px-4 py-6 md:px-6">
            <div>
              <p className="text-sm text-on-surface-variant" data-testid="activity-when">
                <span className="font-mono">{formatInstant(data.activity.start_date, language, ROW_DATE)}</span>
                {' · '}
                {sport}
              </p>
              <ActivityMap activity={data.activity} />
            </div>
            <ActivityChat activity={activity} detail={data} onNavigate={onNavigate} />
            <div
              data-testid="activity-details"
              className="space-y-8 border-0 ghost-border lg:absolute lg:inset-y-0 lg:right-0 lg:!mt-0 lg:flex lg:w-[420px] lg:flex-col lg:gap-8 lg:space-y-0 lg:overflow-y-auto lg:border-l lg:px-6 lg:py-6"
            >
              <Figures detail={data} />
              {splits && <Segments title={t('home.activity.splits')} table={splits} testId="activity-splits" />}
              {laps && <Segments title={t('home.activity.laps')} table={laps} testId="activity-laps" />}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
