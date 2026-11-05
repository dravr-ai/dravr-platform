// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's thread with the Dravr team — read and answered by people, never an LLM, opened from the account menu
// ABOUTME: The team's replies show as the team's bubble; a message sent from a thumbs-down carries that reply (carnet#1392)

import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ImagePlus, X } from 'lucide-react';
import { dayLabelFor, formatMessageTime, localDayKey } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { APP_VERSION } from '@pierre/shared-constants';
import {
  FEEDBACK_IMAGE_MAX_BYTES,
  FEEDBACK_IMAGE_TYPES,
  FEEDBACK_MESSAGE_MAX_CHARS,
  isFeedbackImageType,
  type FeedbackAbout,
  type FeedbackImageUpload,
  type FeedbackMessage,
} from '@pierre/shared-types';
import { DravrLogo } from '../DravrLogo';
import { FilePickerButton } from '../ui';
import MessageBubble from '../chat/MessageBubble';
import DaySeparator from '../chat/DaySeparator';
import { useFeedbackImageUrl, useFeedbackThread } from '../../hooks/useFeedbackThread';

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

/** The screenshot on one of the athlete's own messages once it has loaded, or a line saying it could not be. */
function SentImage({ messageId }: { messageId: string }) {
  const { t } = useTranslation();
  const { url, failed } = useFeedbackImageUrl(messageId);
  if (failed) return <p className="mb-1 text-sm text-error">{t('feedback.imageLoadFailed')}</p>;
  if (!url) return null;
  return <img src={url} alt={t('feedback.imageAlt')} className="mb-1 max-h-64 max-w-full rounded-lg" />;
}

/** A screenshot picked for the next message: what is sent, and what the composer shows. */
interface PickedImage {
  upload: FeedbackImageUpload;
  preview: string;
}

/**
 * A messenger thread with one other side: the Dravr team. Its first row is
 * the team's standing welcome, drawn as any agent row would be; then the
 * athlete's messages and the team's replies, each reply signed "Dravr team"
 * whoever sent it. The team replies from Slack, so the thread re-reads while
 * it is open (useFeedbackThread).
 */
export default function FeedbackThread({ about, openedFrom, onClearAbout }: FeedbackThreadProps) {
  const { t, language } = useTranslation();
  const { thread, send } = useFeedbackThread();
  const [draft, setDraft] = useState('');
  const [image, setImage] = useState<PickedImage | null>(null);
  const [imageError, setImageError] = useState<string | null>(null);
  const endRef = useRef<HTMLDivElement>(null);
  const messages = thread.data?.messages ?? [];

  useEffect(() => {
    endRef.current?.scrollIntoView?.({ block: 'end' });
  }, [messages.length]);

  const body = draft.trim();
  const canSend = (body.length > 0 || image !== null) && !send.isPending;
  const submit = () => {
    if (!canSend) return;
    send.mutate(
      {
        body,
        screen: openedFrom(),
        app_version: APP_VERSION,
        locale: language,
        ...(about ? { about } : {}),
        ...(image ? { image: image.upload } : {}),
      },
      {
        onSuccess: () => {
          setDraft('');
          setImage(null);
          setImageError(null);
          if (about) onClearAbout();
        },
      },
    );
  };

  // A screenshot is checked here as the server checks it, so a refusal is
  // said before anything is sent; the server still reads its own bytes.
  const pickImage = (file: File) => {
    setImageError(null);
    const mediaType = file.type;
    if (!isFeedbackImageType(mediaType)) {
      setImageError(t('feedback.imageWrongType'));
      return;
    }
    if (file.size > FEEDBACK_IMAGE_MAX_BYTES) {
      setImageError(t('feedback.imageTooLarge'));
      return;
    }
    const reader = new FileReader();
    reader.onerror = () => setImageError(t('feedback.imageReadFailed'));
    reader.onload = () => {
      if (typeof reader.result !== 'string') return;
      const preview = reader.result;
      setImage({ preview, upload: { media_type: mediaType, data: preview.slice(preview.indexOf(',') + 1) } });
    };
    reader.readAsDataURL(file);
  };

  const rows: ReactNode[] = [];
  let previousDay = '';
  let previousAuthor: FeedbackMessage['author'] | null = null;
  for (const message of messages) {
    const day = localDayKey(message.created_at);
    const newDay = Boolean(day) && day !== previousDay;
    if (newDay) {
      rows.push(<DaySeparator key={`day-${day}`} day={dayLabelFor(message.created_at, language)} />);
      previousDay = day;
    }
    const team = message.author === 'team';
    const time = formatMessageTime(message.created_at, language);
    // A run of team replies carries the team's name and mark once, as an
    // agent's run does; the athlete's own rows say they were delivered.
    const bubbleProps = team
      ? {
          side: 'assistant' as const,
          authorLabel: t('feedback.title'),
          avatar: <DravrLogo size={24} />,
          groupStart: newDay || previousAuthor !== 'team',
          timestamp: time,
        }
      : { side: 'user' as const, timestamp: `${time} · ${t('feedback.delivered')}` };
    previousAuthor = message.author;
    rows.push(
      <MessageBubble key={message.id} {...bubbleProps}>
        {message.about ? (
          <div className="mb-1">
            <AboutChip>{t('feedback.aboutReply')}</AboutChip>
          </div>
        ) : null}
        {message.has_image ? <SentImage messageId={message.id} /> : null}
        {message.body ? <p className="whitespace-pre-wrap break-words">{message.body}</p> : null}
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
        {image ? (
          <div className="mb-2 flex items-center gap-2" data-testid="feedback-image-preview">
            <img src={image.preview} alt="" className="h-16 w-16 rounded-lg object-cover" />
            <button
              type="button"
              onClick={() => setImage(null)}
              aria-label={t('feedback.removeImage')}
              className="rounded-full p-1 text-outline hover:text-on-surface focus-ring"
            >
              <X className="h-4 w-4" aria-hidden="true" />
            </button>
          </div>
        ) : null}
        {imageError ? (
          <p role="alert" className="mb-2 text-sm text-error">
            {imageError}
          </p>
        ) : null}
        {send.isError ? (
          <p role="alert" className="mb-2 text-sm text-error">
            {t('feedback.sendFailed')}
          </p>
        ) : null}
        <div className="flex items-end gap-2">
          <FilePickerButton
            accept={FEEDBACK_IMAGE_TYPES.join(',')}
            onPick={pickImage}
            aria-label={t('feedback.attachImage')}
            data-testid="feedback-image"
            className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full text-on-surface-variant hover:bg-surface-container-low hover:text-on-surface focus-ring [@media(pointer:coarse)]:h-12 [@media(pointer:coarse)]:w-12"
          >
            <ImagePlus className="h-5 w-5" aria-hidden="true" />
          </FilePickerButton>
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
