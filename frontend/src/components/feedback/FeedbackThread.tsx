// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's thread with the Dravr team — read by people, answered by no LLM, opened from the account menu
// ABOUTME: Opened from a thumbs-down it carries that reply; each message also sends the screen, app version and language

import { useEffect, useRef, useState, type ReactNode } from 'react';
import { X } from 'lucide-react';
import { dayLabelFor, formatMessageTime, localDayKey } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { APP_VERSION } from '@pierre/shared-constants';
import { FEEDBACK_MESSAGE_MAX_CHARS, type FeedbackAbout } from '@pierre/shared-types';
import { DravrLogo } from '../DravrLogo';
import MessageBubble from '../chat/MessageBubble';
import DaySeparator from '../chat/DaySeparator';
import { useFeedbackThread } from '../../hooks/useFeedbackThread';

interface FeedbackThreadProps {
  /** The agent reply the next message is about, when opened from a thumbs-down. */
  about: FeedbackAbout | null;
  /** The route the thread was opened from, read as each message goes out. */
  openedFrom: () => string;
  /** Let go of `about`: the athlete removed it, or the message about it went out. */
  onClearAbout: () => void;
}

/** The chip naming the reply a message is about, in the composer and on a sent message. */
function AboutChip({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full bg-surface-container-lowest px-2.5 py-0.5 text-xs font-medium text-on-surface-variant">
      {children}
    </span>
  );
}

/**
 * A messenger thread with one other side: the Dravr team. Its first row is
 * the team's standing welcome, drawn as any agent row would be; the rest are
 * the athlete's own messages. Only the athlete writes to it, so nothing polls.
 */
export default function FeedbackThread({ about, openedFrom, onClearAbout }: FeedbackThreadProps) {
  const { t, language } = useTranslation();
  const { thread, send } = useFeedbackThread();
  const [draft, setDraft] = useState('');
  const endRef = useRef<HTMLDivElement>(null);
  const messages = thread.data?.messages ?? [];

  useEffect(() => {
    endRef.current?.scrollIntoView?.({ block: 'end' });
  }, [messages.length]);

  const body = draft.trim();
  const canSend = body.length > 0 && !send.isPending;
  const submit = () => {
    if (!canSend) return;
    send.mutate(
      {
        body,
        screen: openedFrom(),
        app_version: APP_VERSION,
        locale: language,
        ...(about ? { about } : {}),
      },
      {
        onSuccess: () => {
          setDraft('');
          if (about) onClearAbout();
        },
      },
    );
  };

  const rows: ReactNode[] = [];
  let previousDay = '';
  for (const message of messages) {
    const day = localDayKey(message.created_at);
    if (day && day !== previousDay) {
      rows.push(<DaySeparator key={`day-${day}`} day={dayLabelFor(message.created_at, language)} />);
      previousDay = day;
    }
    rows.push(
      <MessageBubble
        key={message.id}
        side="user"
        timestamp={`${formatMessageTime(message.created_at, language)} · ${t('feedback.delivered')}`}
      >
        {message.about ? (
          <div className="mb-1">
            <AboutChip>{t('feedback.aboutReply')}</AboutChip>
          </div>
        ) : null}
        <p className="whitespace-pre-wrap break-words">{message.body}</p>
      </MessageBubble>,
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col bg-surface" data-testid="feedback-thread">
      <header className="flex h-16 shrink-0 items-center gap-3 border-b ghost-border px-4 md:px-6">
        <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full border ghost-border bg-surface-container-lowest">
          <DravrLogo size={26} />
        </span>
        <div className="min-w-0">
          <h1 className="truncate font-display text-xl font-semibold text-on-surface">{t('feedback.title')}</h1>
          <p className="truncate text-xs text-outline">{t('feedback.subtitle')}</p>
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-[720px] px-4 pb-4 md:px-6">
          <MessageBubble side="assistant" authorLabel={t('feedback.title')} avatar={<DravrLogo size={24} />}>
            <p className="text-base">{t('feedback.welcome')}</p>
          </MessageBubble>
          {thread.isError ? (
            <p role="alert" className="mt-4 text-sm text-error">
              {t('feedback.loadFailed')}
            </p>
          ) : null}
          {rows}
          <div ref={endRef} />
        </div>
      </div>

      <form
        className="mx-auto w-full max-w-[720px] shrink-0 px-4 pb-4 md:px-6"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        {about ? (
          <div className="mb-2 flex items-center gap-1" data-testid="feedback-about">
            <AboutChip>
              {t('feedback.aboutReply')}
              <button
                type="button"
                onClick={onClearAbout}
                aria-label={t('feedback.removeAboutReply')}
                className="-mr-1 rounded-full p-0.5 text-outline hover:text-on-surface focus-ring"
              >
                <X className="h-3.5 w-3.5" aria-hidden="true" />
              </button>
            </AboutChip>
          </div>
        ) : null}
        {send.isError ? (
          <p role="alert" className="mb-2 text-sm text-error">
            {t('feedback.sendFailed')}
          </p>
        ) : null}
        <div className="flex items-end gap-2">
          <label htmlFor="feedback-composer" className="sr-only">
            {t('feedback.placeholder')}
          </label>
          {/* The thread's composer is the chat composer's filled field, not
              the editorial underline the ui Textarea draws. */}
          {/* eslint-disable-next-line no-restricted-syntax */}
          <textarea
            id="feedback-composer"
            // Marks the field useComposerFocused watches: a phone folds its
            // tab bar away while the athlete types.
            data-composer="true"
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
                event.preventDefault();
                submit();
              }
            }}
            rows={1}
            maxLength={FEEDBACK_MESSAGE_MAX_CHARS}
            placeholder={t('feedback.placeholder')}
            className="max-h-40 flex-1 resize-none rounded-xl bg-surface-container-low px-4 py-2 [@media(pointer:coarse)]:min-h-[48px] [@media(pointer:coarse)]:py-3 text-base text-on-surface placeholder:text-outline focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          />
          <button
            type="submit"
            disabled={!canSend}
            className="btn-primary shrink-0 disabled:opacity-50"
          >
            {t('feedback.send')}
          </button>
        </div>
        <p className="mt-2 text-xs text-outline">{t('feedback.contextNotice')}</p>
      </form>
    </div>
  );
}
