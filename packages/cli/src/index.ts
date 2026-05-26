#!/usr/bin/env bun
import {
  formatDbError,
  getDefaultDbPath,
  migrate,
  withDb,
  openDb,
} from '@luida/core';
import { createRealIntegrations, runSidecar } from '@luida/sidecar';
import { runUi } from '@luida/ui';
import {
  MemoryStore,
  findCandidateInProposals,
  promotePattern,
  reflect,
  runBrain,
  syncRelationshipsFromYaml,
} from '@luida/brain';
import { runMcpServer } from '@luida/mcp';
import { runWebServer } from '@luida/web';
import { readFileSync } from 'node:fs';
import { createRepos } from '@luida/core';
import { Router } from './router';

const router = new Router()
  .register({
    key: 'db init',
    desc: 'tavern.db 초기화·마이그레이션',
    handler: async () => {
      const dbPath = getDefaultDbPath();
      await withDb(async (db) => {
        const result = await migrate(db);
        console.log('🏮 루이다의 술집을 준비했어요.');
        console.log(`   DB: ${dbPath}`);
        if (result.applied.length === 0) {
          console.log(
            `   상태: 이미 최신 (적용 완료 ${result.alreadyApplied.length}건)`,
          );
        } else {
          console.log(`   새로 적용: ${result.applied.join(', ')}`);
        }
      }, dbPath);
    },
  })
  .register({
    key: 'brain start',
    desc: '헤드리스 brain 데몬 시작 (stuck quest 감지, 학습)',
    handler: async (ctx) => {
      const intervalMs = numOpt(ctx.options, 'interval') ?? 60_000;
      const once = boolOpt(ctx.options, 'once');
      console.log(`🧠 brain 시작 (interval=${intervalMs}ms${once ? ', once' : ''})`);
      const handle = await runBrain({ intervalMs, once });
      if (once) {
        console.log('brain tick 1회 완료. 종료.');
        return;
      }
      await new Promise<void>((resolve) => {
        const sig = (): void => {
          handle.stop();
          process.off('SIGINT', sig);
          process.off('SIGTERM', sig);
          resolve();
        };
        process.once('SIGINT', sig);
        process.once('SIGTERM', sig);
      });
    },
  })
  .register({
    key: 'brain reflect',
    desc: '학습 reflect job 1회 실행 (패턴 후보 발굴)',
    handler: async () => {
      const db = openDb();
      const repos = createRepos(db);
      try {
        const memory = new MemoryStore();
        const r = await reflect(repos, memory);
        console.log(
          `🧠 reflect — 후보 ${r.candidates.length}건, markdown ${r.written.length}건, proposal ${r.proposed}건`,
        );
        for (const c of r.candidates) {
          console.log(
            `   • ${c.id} (${(c.confidence * 10).toFixed(1)}/10, ${c.evidence}건)`,
          );
        }
      } finally {
        try {
          repos.close();
        } catch {
          // ignore
        }
        db.close();
      }
    },
  })
  .register({
    key: 'promote-pattern',
    desc: '학습된 패턴 후보를 relationship으로 승격 (기본 disabled, --activate로 즉시 활성화)',
    handler: async (ctx) => {
      const id = ctx.args[0];
      if (!id)
        throw new Error(
          '패턴 id 인자 필요. 예: luida promote-pattern luida-to-agora [--activate]',
        );
      const activate = boolOpt(ctx.options, 'activate');

      const db = openDb();
      const repos = createRepos(db);
      try {
        // C2: events 휘발성에 의존 안 함 — proposal inmail에서 직접 candidate 복원
        let candidate = findCandidateInProposals(repos, id);
        if (!candidate) {
          // fallback: 최근 reflect 재실행해서 찾아봄
          const memory = new MemoryStore();
          const r = await reflect(repos, memory);
          candidate = r.candidates.find((c) => c.id === id) ?? null;
        }
        if (!candidate) {
          console.error(`패턴 후보를 찾을 수 없습니다: ${id}`);
          console.error('최근 proposal inmail에서도 events에서도 후보가 사라졌습니다.');
          process.exit(1);
        }
        const result = promotePattern(repos.relationships, candidate, {
          activate,
        });
        const mode = activate
          ? '활성 (auto_dispatch)'
          : '비활성 (proposal 모드 — --activate로 켜기)';
        console.log(
          `📜 패턴 승급 — ${id} → relationship #${result.relationshipId} · ${mode}`,
        );
      } finally {
        try {
          repos.close();
        } catch {
          // ignore
        }
        db.close();
      }
    },
  })
  .register({
    key: 'sync-rules',
    desc: 'relationships.yaml 파일을 DB에 동기화 (upsert by name)',
    handler: async (ctx) => {
      const path = ctx.args[0];
      if (!path) throw new Error('yaml 파일 경로 인자 필요. 예: luida sync-rules ~/.luida/relationships.yaml');
      const yaml = readFileSync(path, 'utf8');
      const db = openDb();
      try {
        const repos = createRepos(db);
        const result = syncRelationshipsFromYaml(repos.relationships, yaml);
        console.log(
          `📜 룰 동기화 — 신규: ${result.added} · 갱신: ${result.updated} · 실패: ${result.failed}`,
        );
        repos.close();
      } finally {
        db.close();
      }
    },
  })
  .register({
    key: 'mcp start',
    desc: 'MCP server 시작 (stdio JSON-RPC). main pane Claude가 붙음',
    handler: async (ctx) => {
      const me = strOpt(ctx.options, 'me') ?? 'luida';
      // 디버그 메시지는 stderr로 (stdout은 MCP 채널 전용)
      console.error(`🛰  MCP server 시작 — me=${me}`);
      const handle = await runMcpServer({ me });
      await new Promise<void>((resolve) => {
        const sig = (): void => {
          handle.close();
          process.off('SIGINT', sig);
          process.off('SIGTERM', sig);
          resolve();
        };
        process.once('SIGINT', sig);
        process.once('SIGTERM', sig);
      });
    },
  })
  .register({
    key: 'web',
    desc: 'Web 대시보드 데브 서버 (Luida Tavern HTML — 향후 Tauri로 래핑)',
    handler: async (ctx) => {
      const port = numOpt(ctx.options, 'port') ?? 4321;
      const handle = await runWebServer({ port });
      console.log(`🍺 주점 데스크탑(beta) — ${handle.url}`);
      console.log(`   브라우저에서 열거나 Tauri 윈도우로 래핑하세요.`);
      await new Promise<void>((resolve) => {
        const sig = (): void => {
          handle.stop().finally(() => {
            process.off('SIGINT', sig);
            process.off('SIGTERM', sig);
            resolve();
          });
        };
        process.once('SIGINT', sig);
        process.once('SIGTERM', sig);
      });
    },
  })
  .register({
    key: 'ui',
    desc: 'TUI 대시보드 — 술집 화면을 띄움',
    handler: async (ctx) => {
      const interval = numOpt(ctx.options, 'interval') ?? 1_000;
      await runUi({ intervalMs: interval });
    },
  })
  .register({
    key: 'sidecar',
    desc: '모험가 sidecar 데몬 시작 (cmux pane별 1개)',
    handler: async (ctx) => {
      const me = strOpt(ctx.options, 'me', 'n');
      const repoPath = strOpt(ctx.options, 'repo') ?? process.cwd();
      const workspaceId =
        strOpt(ctx.options, 'workspace') ??
        process.env.CMUX_WORKSPACE_ID ??
        '';
      const surfaceId =
        strOpt(ctx.options, 'surface') ??
        process.env.CMUX_SURFACE_ID ??
        '';
      const once = boolOpt(ctx.options, 'once');
      const autoPr = boolOpt(ctx.options, 'auto-pr');
      const intervalMs = numOpt(ctx.options, 'interval') ?? 10_000;

      if (!me) {
        throw new Error('--me <name> 필수');
      }
      if (!workspaceId || !surfaceId) {
        throw new Error(
          'CMUX_WORKSPACE_ID / CMUX_SURFACE_ID env 또는 --workspace/--surface 필요',
        );
      }

      console.log(`🎒 sidecar 시작 — me=${me}, surface=${surfaceId}`);
      const result = await runSidecar({
        me,
        repoPath,
        workspaceId,
        surfaceId,
        once,
        autoCreatePr: autoPr,
        intervalMs,
        integrations: createRealIntegrations(),
      });

      if (once) {
        console.log(`처리: ${result.processedOnce}건. 종료.`);
        return;
      }

      console.log(`polling 시작 (${intervalMs}ms). Ctrl-C로 종료.`);
      await new Promise<void>((resolve) => {
        const onSignal = (): void => {
          result.close();
          process.off('SIGINT', onSignal);
          process.off('SIGTERM', onSignal);
          resolve();
        };
        process.once('SIGINT', onSignal);
        process.once('SIGTERM', onSignal);
      });
    },
  });

function strOpt(
  opts: Record<string, string | boolean>,
  ...keys: string[]
): string | undefined {
  for (const k of keys) {
    const v = opts[k];
    if (typeof v === 'string') return v;
  }
  return undefined;
}

function boolOpt(
  opts: Record<string, string | boolean>,
  key: string,
): boolean {
  const v = opts[key];
  return v === true || v === 'true' || v === '1';
}

function numOpt(
  opts: Record<string, string | boolean>,
  key: string,
): number | undefined {
  const v = opts[key];
  if (typeof v === 'string') {
    const n = Number(v);
    return Number.isFinite(n) ? n : undefined;
  }
  return undefined;
}

async function main(): Promise<void> {
  const argv = process.argv.slice(2);

  if (argv.length === 0 || argv[0] === '--help' || argv[0] === '-h') {
    console.log(router.formatHelp());
    console.log(`
Env:
  LUIDA_DB_PATH          tavern.db 경로 override
  LUIDA_MIGRATIONS_DIR   마이그레이션 SQL 디렉터리 override
  CMUX_WORKSPACE_ID      cmux pane이 sidecar에 노출하는 workspace id
  CMUX_SURFACE_ID        cmux pane이 sidecar에 노출하는 surface id
  LUIDA_DEBUG=1          에러 시 stack trace 출력
`);
    return;
  }

  const matched = router.resolve(argv);
  if (!matched) {
    console.error(`알 수 없는 명령: ${argv.join(' ')}`);
    console.log(router.formatHelp());
    process.exit(1);
  }

  await matched.command.handler(matched.ctx);
}

try {
  await main();
} catch (err) {
  console.error(`에러: ${formatDbError(err)}`);
  if (process.env.LUIDA_DEBUG === '1' && err instanceof Error && err.stack) {
    console.error(err.stack);
  }
  process.exit(1);
}
