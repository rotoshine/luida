#!/usr/bin/env bun
// Claude Stop hook entry. Phase 1에서는 stub.
// 실제 quest 완료 처리 / relationship 평가는 Phase 3에서 정착.
//
// hook이 받는 stdin 형식: { session_id, transcript_path, ... } (Claude Code 표준)
// 환경변수: $CLAUDE_PROJECT_DIR, $LUIDA_SESSION_NAME (사용자가 .claude/settings.json에 설정)

const me = process.env.LUIDA_SESSION_NAME;
if (!me) {
  console.error('Stop hook: LUIDA_SESSION_NAME 미설정. skip.');
  process.exit(0);
}

// Phase 1 stub: 단순 로그만 남기고 종료.
// (실제 quest_completed 이벤트 발행은 Phase 3에서 적용)
console.error(`[luida stop-hook] turn end for adventurer=${me}`);
process.exit(0);
