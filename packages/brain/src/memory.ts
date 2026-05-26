// Luida brain의 학습 메모리 (markdown 파일 기반).
//   ~/.luida/memory/
//     chronicle.md             — 시간순 누적 (모험 일지)
//     projects/<name>.md       — 모험가별 메모
//     patterns/YYYY-MM-DD-*.md — 학습 패턴 후보 (Phase 5에서 본격화)

import { existsSync, mkdirSync, readFileSync, writeFileSync, readdirSync, appendFileSync, statSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';

export function getMemoryDir(): string {
  return process.env.LUIDA_MEMORY_DIR ?? join(homedir(), '.luida', 'memory');
}

function ensureDir(path: string): void {
  if (!existsSync(path)) mkdirSync(path, { recursive: true });
}

export type RecallScope = 'chronicle' | 'project' | 'patterns' | 'all';

export type RecallResult = {
  chronicle?: string;
  project?: string;
  patterns?: { name: string; content: string }[];
};

export type RecordType = 'chronicle' | 'project' | 'pattern';

export type RecordInput = {
  type: RecordType;
  /** project/pattern일 때 필요. chronicle은 무시 */
  name?: string;
  content: string;
};

export class MemoryStore {
  constructor(private readonly baseDir = getMemoryDir()) {
    ensureDir(this.baseDir);
    ensureDir(join(this.baseDir, 'projects'));
    ensureDir(join(this.baseDir, 'patterns'));
  }

  chroniclePath(): string {
    return join(this.baseDir, 'chronicle.md');
  }

  projectPath(name: string): string {
    return join(this.baseDir, 'projects', `${sanitize(name)}.md`);
  }

  patternPath(name: string): string {
    return join(this.baseDir, 'patterns', `${sanitize(name)}.md`);
  }

  /**
   * chronicle에 한 줄 append (자동으로 timestamp + 개행 추가).
   * Phase 5: 파일이 일정 크기 초과 시 월 단위 rotation
   *   chronicle.md → chronicle.YYYY-MM.md
   */
  appendChronicle(line: string, now: number = Date.now()): void {
    const ts = new Date(now).toISOString();
    this.rotateChronicleIfLarge(now);
    appendFileSync(this.chroniclePath(), `\n## ${ts}\n${line}\n`);
  }

  /**
   * chronicle 파일이 너무 커지면 월 아카이브로 옮긴다.
   *
   * 원자성 (M3 대응):
   *  - 새 아카이브가 없으면 `renameSync`로 원자적 이동 (같은 fs 내 atomic)
   *  - 이미 같은 달 아카이브가 있으면 임시 파일로 합친 뒤 rename → swap
   *  - 어떤 단계든 실패하면 원본을 비우지 않음 (데이터 손실 차단)
   */
  private rotateChronicleIfLarge(
    now: number,
    maxBytes = 2 * 1024 * 1024, // 2MB
  ): void {
    const p = this.chroniclePath();
    if (!existsSync(p)) return;
    let size = 0;
    try {
      size = statSync(p).size;
    } catch {
      return;
    }
    if (size < maxBytes) return;

    const ym = new Date(now).toISOString().slice(0, 7);
    const archive = join(this.baseDir, `chronicle.${ym}.md`);
    const tmp = join(this.baseDir, `chronicle.${ym}.md.tmp-${now}`);

    try {
      if (!existsSync(archive)) {
        // 가장 단순한 케이스: 원자적 rename. 본 파일은 사라지므로 자동으로 "비움" 효과
        const { renameSync } = require('node:fs') as typeof import('node:fs');
        renameSync(p, archive);
        return;
      }
      // 아카이브가 이미 있으면 임시 파일로 합치고 swap
      writeFileSync(tmp, readFileSync(archive, 'utf8') + '\n\n' + readFileSync(p, 'utf8'));
      const { renameSync } = require('node:fs') as typeof import('node:fs');
      renameSync(tmp, archive);
      // 원본은 합쳐졌으므로 안전하게 비움
      writeFileSync(p, '');
    } catch {
      // 실패 시 원본 보존
    }
  }

  /** 프로젝트 메모 전체 덮어쓰기 */
  writeProject(name: string, content: string): void {
    ensureDir(dirname(this.projectPath(name)));
    writeFileSync(this.projectPath(name), content);
  }

  /**
   * 패턴 추가. 같은 name이 이미 있으면 timestamp suffix 자동 부여 (M4 대응).
   */
  writePattern(name: string, content: string, now: number = Date.now()): string {
    ensureDir(dirname(this.patternPath(name)));
    let finalName = name;
    if (existsSync(this.patternPath(name))) {
      const suffix = String(now).slice(-6);
      finalName = `${name}-${suffix}`;
    }
    writeFileSync(this.patternPath(finalName), content);
    return finalName;
  }

  recall(
    scope: RecallScope,
    opts: { project?: string; limit?: number } = {},
  ): RecallResult {
    const result: RecallResult = {};

    if (scope === 'chronicle' || scope === 'all') {
      const p = this.chroniclePath();
      if (existsSync(p)) {
        const text = readFileSync(p, 'utf8');
        const limit = opts.limit ?? 4000;
        result.chronicle =
          text.length <= limit ? text : text.slice(-limit);
      }
    }

    if ((scope === 'project' || scope === 'all') && opts.project) {
      const p = this.projectPath(opts.project);
      if (existsSync(p)) result.project = readFileSync(p, 'utf8');
    }

    if (scope === 'patterns' || scope === 'all') {
      const dir = join(this.baseDir, 'patterns');
      if (existsSync(dir)) {
        const files = readdirSync(dir)
          .filter((f) => f.endsWith('.md'))
          .sort()
          .reverse(); // 최신 우선
        const limit = opts.limit ?? 10;
        result.patterns = files.slice(0, limit).map((f) => ({
          name: f,
          content: readFileSync(join(dir, f), 'utf8'),
        }));
      }
    }

    return result;
  }

  record(input: RecordInput, now: number = Date.now()): void {
    switch (input.type) {
      case 'chronicle':
        this.appendChronicle(input.content, now);
        break;
      case 'project':
        if (!input.name) throw new Error('project 메모는 name 필수');
        this.writeProject(input.name, input.content);
        break;
      case 'pattern':
        if (!input.name) throw new Error('pattern 메모는 name 필수');
        this.writePattern(input.name, input.content);
        break;
    }
  }
}

function sanitize(name: string): string {
  return name.replace(/[^A-Za-z0-9_\-가-힣]/g, '_').slice(0, 100);
}
