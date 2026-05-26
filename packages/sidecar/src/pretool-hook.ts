#!/usr/bin/env bun
// PreToolUse hook 진입점.
//
// Claude Code의 `.claude/settings.json`에 다음과 같이 등록:
//   {
//     "hooks": {
//       "PreToolUse": [
//         { "hooks": [{ "type": "command",
//             "command": "bun run /Users/roto/workspace/luida/packages/sidecar/src/pretool-hook.ts" }]}
//       ]
//     }
//   }
//
// 환경변수:
//   LUIDA_WORKTREE_ROOT  worker가 동작해야 할 worktree path (필수)
//   LUIDA_GUARD_DISABLED 디버그용 우회 (값='1'이면 항상 allow)
//
// 표준 종료 코드:
//   0  allow (조용히 통과)
//   2  block (stdout이 사용자에게 표시됨)

import {
  DEFAULT_SECRET_PATTERNS,
  defaultBlockedPaths,
  evaluateHook,
  type GuardInput,
} from './guards';

async function readStdin(): Promise<string> {
  let buf = '';
  for await (const chunk of process.stdin) {
    buf += chunk;
  }
  return buf;
}

async function main(): Promise<void> {
  if (process.env.LUIDA_GUARD_DISABLED === '1') {
    process.exit(0);
  }

  const root = process.env.LUIDA_WORKTREE_ROOT;
  if (!root) {
    // 환경변수 미설정 — 차단보단 통과 (warning만 stderr)
    console.error(
      '[luida pretool-hook] LUIDA_WORKTREE_ROOT 미설정 — 가드 비활성',
    );
    process.exit(0);
  }

  let input: GuardInput;
  try {
    const text = await readStdin();
    input = JSON.parse(text) as GuardInput;
  } catch {
    process.exit(0); // 파싱 실패도 통과 (Claude hook 표준)
  }

  const decision = evaluateHook(input, {
    worktreeRoot: root,
    blockedPaths: defaultBlockedPaths(),
    secretPatterns: DEFAULT_SECRET_PATTERNS,
  });

  if (decision.allow) {
    process.exit(0);
  }

  console.error(`🛡  luida 가드 차단: ${decision.reason}`);
  console.log(`Luida 가드: ${decision.reason}`);
  process.exit(2);
}

main().catch((err) => {
  console.error('[luida pretool-hook] error:', err);
  // 가드 자체 실패는 통과 (denial-of-service 방지)
  process.exit(0);
});
