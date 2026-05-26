import type {
  WorkerRunner,
  WorkerSpawnOptions,
  WorkerStreamEvent,
} from '@luida/core';

/**
 * `claude -p --output-format stream-json` headless worker를 실행한다.
 * stdout NDJSON을 한 줄씩 파싱해서 WorkerStreamEvent로 yield.
 *
 * 견고성 (Phase 1 리뷰 C3 대응):
 *  - stderr를 백그라운드 drain하여 파이프 버퍼 가득참으로 인한 hang 방지
 *  - finally에서 proc.kill() 보장 (호출자가 generator를 중도 close해도 worker 정리)
 *  - exit code != 0 시 stderr 일부를 error 이벤트에 포함
 *  - parseStreamLine은 export하여 단위 테스트 가능
 */
export class ClaudeWorkerRunner implements WorkerRunner {
  constructor(private readonly bin = 'claude') {}

  async *spawn(
    opts: WorkerSpawnOptions,
  ): AsyncIterable<WorkerStreamEvent> {
    const args = ['-p', '--output-format', 'stream-json'];
    if (opts.sessionId) {
      args.push('--session-id', opts.sessionId);
    }
    args.push(opts.brief);

    const proc = Bun.spawn([this.bin, ...args], {
      cwd: opts.cwd,
      env: { ...process.env, ...(opts.env ?? {}) },
      stdout: 'pipe',
      stderr: 'pipe',
    });

    // stderr 백그라운드 drain — 가득 차서 child가 block되는 것 방지
    const stderrPromise = new Response(proc.stderr)
      .text()
      .catch((): string => '');

    const reader = proc.stdout.getReader();
    const decoder = new TextDecoder();
    let buffer = '';
    let killed = false;
    let streamError: string | null = null;

    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });

        let nl: number;
        while ((nl = buffer.indexOf('\n')) >= 0) {
          const line = buffer.slice(0, nl).trim();
          buffer = buffer.slice(nl + 1);
          if (!line) continue;
          const ev = parseStreamLine(line);
          if (ev) yield ev;
        }
      }

      if (buffer.trim()) {
        const ev = parseStreamLine(buffer.trim());
        if (ev) yield ev;
      }
    } catch (err) {
      streamError = (err as Error)?.message ?? String(err);
    } finally {
      if (!killed) {
        try {
          proc.kill();
        } catch {
          // 이미 죽었으면 무시
        }
        killed = true;
      }
    }

    const exit = await proc.exited;
    const stderr = await stderrPromise;
    if (streamError) {
      yield {
        kind: 'error',
        message: `stream read failed: ${streamError}`,
      };
    }
    if (exit !== 0) {
      const tail = stderr.trim().slice(-500);
      yield {
        kind: 'error',
        message: `claude exited ${exit}${tail ? `: ${tail}` : ''}`,
      };
    }
  }
}

export function parseStreamLine(line: string): WorkerStreamEvent | null {
  try {
    const obj = JSON.parse(line) as Record<string, unknown>;
    const t = obj.type;
    if (t === 'text' && typeof obj.text === 'string') {
      return { kind: 'text', text: obj.text };
    }
    if (t === 'tool_use' && typeof obj.name === 'string') {
      return { kind: 'tool_use', name: obj.name, input: obj.input };
    }
    if (t === 'result') {
      // Claude Code stream-json의 정확한 success 필드는 변동 가능.
      // is_error=true 또는 subtype='error'를 실패로 간주.
      const isError =
        obj.is_error === true ||
        obj.subtype === 'error' ||
        obj.success === false;
      return {
        kind: 'result',
        success: !isError,
        summary:
          typeof obj.summary === 'string'
            ? obj.summary
            : typeof obj.result === 'string'
              ? obj.result
              : undefined,
      };
    }
    if (t === 'system' && typeof obj.subtype === 'string') {
      return { kind: 'system', subtype: obj.subtype, payload: obj };
    }
    if (t === 'error' && typeof obj.message === 'string') {
      return { kind: 'error', message: obj.message };
    }
    return null;
  } catch {
    return null;
  }
}
