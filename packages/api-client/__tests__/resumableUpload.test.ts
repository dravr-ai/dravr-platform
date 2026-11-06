// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit cover for the resumable uploader: chunks and Content-Range, the offset query, resume after a failed chunk, giving up
// ABOUTME: A fake session keeps the store's committed bytes, so a resume is judged by what it really holds, not by the calls made

import { describe, expect, it, vi } from 'vitest';
import {
  ExportUploadError,
  UPLOAD_MAX_STALLED_ATTEMPTS,
  committedLength,
  uploadToSession,
  type UploadBytes,
} from '../src/core/resumableUpload';
import { uploadExport } from '../src/core/exportUpload';

const URL = 'https://storage.example.com/upload/session-secret-token';
const CHUNK = 4;

function fileOf(text: string): UploadBytes & { reads: [number, number][]; closed: boolean } {
  const bytes = new TextEncoder().encode(text);
  const file = {
    size: bytes.length,
    reads: [] as [number, number][],
    closed: false,
    read: async (start: number, end: number) => {
      file.reads.push([start, end]);
      return bytes.slice(start, end);
    },
    close: () => {
      file.closed = true;
    },
  };
  return file;
}

function answer(status: number, range?: string): Response {
  return {
    status,
    headers: { get: (name: string) => (name === 'Range' ? range ?? null : null) },
  } as unknown as Response;
}

/** A session that commits what it is sent, and fails the requests `script` names by their order. */
function session(total: number, script: Record<number, () => Response | never> = {}, startAt = 0) {
  let committed = startAt;
  const ranges: string[] = [];
  const held: number[] = [];
  const fetchImpl = vi.fn(async (_url: unknown, init: RequestInit) => {
    const call = fetchImpl.mock.calls.length;
    const contentRange = (init.headers as Record<string, string>)['Content-Range'];
    ranges.push(contentRange);
    const scripted = script[call];
    if (scripted) return scripted();
    if (contentRange.startsWith('bytes */')) {
      if (committed >= total) return answer(200);
      return committed === 0 ? answer(308) : answer(308, `bytes=0-${committed - 1}`);
    }
    const match = /^bytes (\d+)-(\d+)\/(\d+)$/.exec(contentRange);
    if (!match) throw new Error(`bad range ${contentRange}`);
    const [start, end] = [Number(match[1]), Number(match[2])];
    if (start !== committed) return answer(503);
    for (const byte of init.body as Uint8Array) held.push(byte);
    committed = end + 1;
    return committed >= total ? answer(201) : answer(308, `bytes=0-${committed - 1}`);
  });
  return { fetchImpl: fetchImpl as unknown as typeof fetch, ranges, held, calls: fetchImpl, committed: () => committed };
}

const noWait = () => Promise.resolve();

function text(bytes: number[]): string {
  return new TextDecoder().decode(new Uint8Array(bytes));
}

describe('committedLength', () => {
  it('reads the end of the range, and nothing committed when there is none', () => {
    expect(committedLength('bytes=0-41')).toBe(42);
    expect(committedLength(null)).toBe(0);
    expect(committedLength('garbage')).toBe(0);
  });
});

describe('uploadToSession', () => {
  it('asks where the session stands, then sends the file in ranged chunks', async () => {
    const file = fileOf('abcdefghij');
    const fake = session(10);
    const progress: number[] = [];
    await uploadToSession({
      sessionUrl: URL,
      file,
      chunkBytes: CHUNK,
      fetchImpl: fake.fetchImpl,
      wait: noWait,
      onProgress: (sent) => progress.push(sent),
    });
    expect(fake.ranges).toEqual(['bytes */10', 'bytes 0-3/10', 'bytes 4-7/10', 'bytes 8-9/10']);
    expect(text(fake.held)).toBe('abcdefghij');
    expect(progress).toEqual([0, 4, 8, 10]);
    expect(file.reads).toEqual([[0, 4], [4, 8], [8, 10]]);
  });

  it('resumes where the store stands after a restart, reading nothing before it', async () => {
    const file = fileOf('abcdefghij');
    const fake = session(10, {}, 4);
    await uploadToSession({ sessionUrl: URL, file, chunkBytes: CHUNK, fetchImpl: fake.fetchImpl, wait: noWait });
    expect(fake.ranges).toEqual(['bytes */10', 'bytes 4-7/10', 'bytes 8-9/10']);
    expect(file.reads[0]).toEqual([4, 8]);
  });

  it('asks the store again after a failed chunk and continues from what it committed', async () => {
    const file = fileOf('abcdefghij');
    const fake = session(10, { 3: () => answer(503) });
    await uploadToSession({ sessionUrl: URL, file, chunkBytes: CHUNK, fetchImpl: fake.fetchImpl, wait: noWait });
    expect(fake.ranges).toEqual([
      'bytes */10',
      'bytes 0-3/10',
      'bytes 4-7/10', // refused: the store has only the first chunk
      'bytes */10',
      'bytes 4-7/10',
      'bytes 8-9/10',
    ]);
    expect(text(fake.held)).toBe('abcdefghij');
  });

  it('survives a network failure on a chunk the same way', async () => {
    const file = fileOf('abcdefgh');
    const fake = session(8, {
      2: () => {
        throw new TypeError('network');
      },
    });
    await uploadToSession({ sessionUrl: URL, file, chunkBytes: CHUNK, fetchImpl: fake.fetchImpl, wait: noWait });
    expect(text(fake.held)).toBe('abcdefgh');
  });

  it('takes a partial commit: the next chunk starts where the store says, not where the last one ended', async () => {
    const file = fileOf('abcdefgh');
    let call = 0;
    const fetchImpl = vi.fn(async () => {
      call += 1;
      if (call === 1) return answer(308);
      if (call === 2) return answer(308, 'bytes=0-1');
      return answer(200);
    });
    await uploadToSession({ sessionUrl: URL, file, chunkBytes: CHUNK, fetchImpl: fetchImpl as unknown as typeof fetch, wait: noWait });
    expect(file.reads).toEqual([[0, 4], [2, 6]]);
  });

  it('finishes at once when the store already holds the whole object', async () => {
    const file = fileOf('abcd');
    const fake = session(4, {}, 4);
    await uploadToSession({ sessionUrl: URL, file, chunkBytes: CHUNK, fetchImpl: fake.fetchImpl, wait: noWait });
    expect(file.reads).toEqual([]);
  });

  it('waits longer each time nothing advances', async () => {
    const wait = vi.fn(noWait);
    const fetchImpl = vi.fn(async () => answer(503));
    await expect(
      uploadToSession({ sessionUrl: URL, file: fileOf('abcd'), fetchImpl: fetchImpl as unknown as typeof fetch, wait }),
    ).rejects.toMatchObject({ failure: 'stalled' });
    expect(wait.mock.calls.map(([ms]) => ms)).toEqual([1000, 2000, 4000, 8000]);
    expect(fetchImpl).toHaveBeenCalledTimes(UPLOAD_MAX_STALLED_ATTEMPTS);
  });

  it('gives up rather than looping when the store hides how much it holds', async () => {
    const file = fileOf('abcdefgh');
    const fetchImpl = vi.fn(async () => answer(308));
    await expect(
      uploadToSession({ sessionUrl: URL, file, chunkBytes: CHUNK, fetchImpl: fetchImpl as unknown as typeof fetch, wait: noWait }),
    ).rejects.toMatchObject({ failure: 'stalled' });
    expect(fetchImpl.mock.calls.length).toBeLessThanOrEqual(UPLOAD_MAX_STALLED_ATTEMPTS * 2 + 1);
  });

  it('progress that resumes the advance clears the count of stalled attempts', async () => {
    const file = fileOf('abcdefghijkl');
    const script: Record<number, () => Response> = {};
    // A failure before each chunk, one fewer in a row than would give up each time.
    const fake = session(12, script);
    let calls = 0;
    const flaky = vi.fn(async (url: unknown, init: RequestInit) => {
      calls += 1;
      if (calls % 3 === 0) return answer(503);
      return (fake.fetchImpl as unknown as (u: unknown, i: RequestInit) => Promise<Response>)(url, init);
    });
    await uploadToSession({ sessionUrl: URL, file, chunkBytes: CHUNK, fetchImpl: flaky as unknown as typeof fetch, wait: noWait });
    expect(text(fake.held)).toBe('abcdefghijkl');
  });

  it.each([404, 410])('says the session is gone on %i', async (status) => {
    const fetchImpl = vi.fn(async () => answer(status));
    await expect(
      uploadToSession({ sessionUrl: URL, file: fileOf('abcd'), fetchImpl: fetchImpl as unknown as typeof fetch, wait: noWait }),
    ).rejects.toMatchObject({ failure: 'gone' });
  });

  it('stops on a refusal retrying cannot change', async () => {
    const fetchImpl = vi.fn(async () => answer(400));
    await expect(
      uploadToSession({ sessionUrl: URL, file: fileOf('abcd'), fetchImpl: fetchImpl as unknown as typeof fetch, wait: noWait }),
    ).rejects.toMatchObject({ failure: 'refused' });
    expect(fetchImpl).toHaveBeenCalledTimes(1);
  });

  it('stops at an abort without retrying', async () => {
    const controller = new AbortController();
    const fetchImpl = vi.fn(async () => {
      controller.abort();
      throw new DOMException('aborted', 'AbortError');
    });
    await expect(
      uploadToSession({
        sessionUrl: URL,
        file: fileOf('abcd'),
        signal: controller.signal,
        fetchImpl: fetchImpl as unknown as typeof fetch,
        wait: noWait,
      }),
    ).rejects.toThrow('aborted');
    expect(fetchImpl).toHaveBeenCalledTimes(1);
  });

  it('names no session URL in its errors, and sends no credentials to the store', async () => {
    const fetchImpl = vi.fn(async (_url: unknown, _init: RequestInit) => answer(403));
    const error = await uploadToSession({
      sessionUrl: URL,
      file: fileOf('abcd'),
      fetchImpl: fetchImpl as unknown as typeof fetch,
      wait: noWait,
    }).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ExportUploadError);
    expect(String((error as Error).message)).not.toContain('session-secret-token');
    expect(fetchImpl.mock.calls[0][1].credentials).toBe('omit');
  });
});

describe('uploadExport', () => {
  const done = { id: 'job-1', status: 'downloading' };

  function api(target: unknown) {
    return {
      startUpload: vi.fn().mockResolvedValue({ import: { id: 'job-1' }, upload: target }),
      getUploadSession: vi.fn().mockResolvedValue(target),
      writeUpload: vi.fn().mockResolvedValue(undefined),
      finishUpload: vi.fn().mockResolvedValue(done),
    };
  }

  it('opens an upload of the file size, writes it to the session and finishes', async () => {
    const client = api({ kind: 'session', url: URL });
    const file = fileOf('abcd');
    const fake = session(4);
    const opened = vi.fn();
    const result = await uploadExport({
      api: client as never,
      source: 'apple_health_archive',
      file,
      waitingImportId: null,
      onOpened: opened,
      fetchImpl: fake.fetchImpl,
      wait: noWait,
    });
    expect(client.startUpload).toHaveBeenCalledWith('apple_health_archive', 4);
    expect(client.getUploadSession).not.toHaveBeenCalled();
    expect(opened).toHaveBeenCalledOnce();
    expect(text(fake.held)).toBe('abcd');
    expect(client.finishUpload).toHaveBeenCalledWith('apple_health_archive', 'job-1');
    expect(result).toBe(done);
    expect(file.closed).toBe(true);
  });

  it('reads back the session of an import that waits rather than opening another', async () => {
    const client = api({ kind: 'session', url: URL });
    const fake = session(4, {}, 2);
    await uploadExport({
      api: client as never,
      source: 'apple_health_archive',
      file: fileOf('abcd'),
      waitingImportId: 'job-7',
      fetchImpl: fake.fetchImpl,
      wait: noWait,
    });
    expect(client.startUpload).not.toHaveBeenCalled();
    expect(client.getUploadSession).toHaveBeenCalledWith('apple_health_archive', 'job-7');
    expect(client.finishUpload).toHaveBeenCalledWith('apple_health_archive', 'job-7');
  });

  it('sends the whole file through the API where the store takes no direct writes', async () => {
    const client = api({ kind: 'through_api' });
    const fetchImpl = vi.fn();
    await uploadExport({
      api: client as never,
      source: 'apple_health_archive',
      file: fileOf('abcd'),
      waitingImportId: null,
      fetchImpl: fetchImpl as unknown as typeof fetch,
    });
    expect(client.writeUpload).toHaveBeenCalledOnce();
    expect(fetchImpl).not.toHaveBeenCalled();
    expect(client.finishUpload).toHaveBeenCalledOnce();
  });

  it('does not say the upload landed when it stopped, and still releases the file', async () => {
    const client = api({ kind: 'session', url: URL });
    const file = fileOf('abcd');
    const fetchImpl = vi.fn(async () => answer(404));
    await expect(
      uploadExport({
        api: client as never,
        source: 'apple_health_archive',
        file,
        waitingImportId: null,
        fetchImpl: fetchImpl as unknown as typeof fetch,
        wait: noWait,
      }),
    ).rejects.toMatchObject({ failure: 'gone' });
    expect(client.finishUpload).not.toHaveBeenCalled();
    expect(file.closed).toBe(true);
  });
});
