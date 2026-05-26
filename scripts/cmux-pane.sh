#!/usr/bin/env bash
# cmux pane 시작 헬퍼 — sidecar를 백그라운드로 띄우고 claude를 foreground로 실행.
#
# 사용법:
#   cmux-pane.sh <session-name> [--auto-pr]
#
# 예:
#   cmux-pane.sh agora
#   cmux-pane.sh admin --auto-pr

set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "Usage: $0 <session-name> [--auto-pr]" >&2
  exit 1
fi

SESSION_NAME="$1"
shift

# cmux 환경변수 검증
if [[ -z "${CMUX_WORKSPACE_ID:-}" || -z "${CMUX_SURFACE_ID:-}" ]]; then
  echo "⚠  CMUX_WORKSPACE_ID / CMUX_SURFACE_ID 미설정. cmux pane 안에서 실행했나요?" >&2
fi

# 로그 디렉터리 보장
LOG_DIR="${HOME}/.luida/log"
mkdir -p "$LOG_DIR"

# luida CLI 경로 — 사용자 환경에 맞게 조정
LUIDA_BIN="${LUIDA_BIN:-luida}"

# sidecar를 백그라운드로
echo "🎒 sidecar 시작 — me=$SESSION_NAME · log=$LOG_DIR/$SESSION_NAME.log"
nohup "$LUIDA_BIN" sidecar --me "$SESSION_NAME" "$@" \
  > "$LOG_DIR/$SESSION_NAME.log" 2>&1 &
SIDECAR_PID=$!
echo "   pid=$SIDECAR_PID"

# 트랩: pane 종료시 sidecar도 정리
trap "kill $SIDECAR_PID 2>/dev/null || true" EXIT INT TERM

# claude를 foreground로
exec claude
