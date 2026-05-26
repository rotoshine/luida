// Phase D: worker 안전 가드.
//
// Claude Code의 PreToolUse hook에서 호출돼, worker가 worktree 밖 파일에
// 접근하거나 시크릿 파일을 만지려 할 때 차단한다.
//
// hook 표준 stdin format: { tool_name, tool_input: { file_path|path, ... }, session_id, ... }
// 표준 응답: exit 0 = allow, exit 2 + stdout decision = block

import { resolve, sep } from 'node:path';

export type GuardDecision = {
  allow: boolean;
  reason?: string;
};

export type GuardInput = {
  /** PreToolUse hook이 전달하는 tool 이름 */
  tool_name?: string;
  tool_input?: {
    file_path?: string;
    path?: string;
    pattern?: string;
    command?: string;
    [k: string]: unknown;
  };
  /** worker의 cwd (worktree path) */
  cwd?: string;
};

export type GuardOpts = {
  /** worker가 동작해야 할 worktree 루트 — 이 경로 밖은 차단 */
  worktreeRoot: string;
  /** 추가로 차단할 절대 경로 prefix (예: $HOME/.ssh, $HOME/.aws) */
  blockedPaths?: string[];
  /** 차단할 파일명 패턴 (basename 매칭) */
  secretPatterns?: RegExp[];
};

export const DEFAULT_SECRET_PATTERNS: RegExp[] = [
  /^\.env(\..+)?$/, // .env, .env.local, .env.production
  /\.pem$/i,
  /\.key$/i,
  /\.p12$/i,
  /\.pfx$/i,
  /credentials\.json$/i,
  /service-account.*\.json$/i,
  /^id_rsa$/,
  /^id_ed25519$/,
];

export function defaultBlockedPaths(): string[] {
  const home = process.env.HOME;
  if (!home) return [];
  return [
    `${home}/.ssh`,
    `${home}/.aws`,
    `${home}/.gnupg`,
    `${home}/.config/gh`, // gh CLI token
  ];
}

/**
 * 단일 file path가 worktree 안에 있고 시크릿이 아닌지 검사.
 */
export function checkPath(filePath: string, opts: GuardOpts): GuardDecision {
  if (!filePath) return { allow: true };

  // 1) absolute path 정규화 — worktree 밖이면 차단
  const abs = resolve(filePath);
  const root = resolve(opts.worktreeRoot);
  if (!abs.startsWith(root + sep) && abs !== root) {
    return {
      allow: false,
      reason: `worktree 밖 경로 접근 차단: ${abs} (root: ${root})`,
    };
  }

  // 2) 추가 차단 경로
  for (const blocked of opts.blockedPaths ?? []) {
    if (abs.startsWith(resolve(blocked))) {
      return { allow: false, reason: `차단된 경로: ${blocked}` };
    }
  }

  // 3) 시크릿 파일명
  const base = abs.split(sep).pop() ?? '';
  for (const re of opts.secretPatterns ?? DEFAULT_SECRET_PATTERNS) {
    if (re.test(base)) {
      return { allow: false, reason: `시크릿 파일 차단: ${base}` };
    }
  }

  return { allow: true };
}

/**
 * PreToolUse hook 진입점. stdin JSON을 받아 allow/block 결정.
 *
 * Claude Code 표준:
 *  - exit 0: allow (stdout 무시)
 *  - exit 2: block (stdout이 사용자에게 표시됨)
 */
export function evaluateHook(input: GuardInput, opts: GuardOpts): GuardDecision {
  const t = input.tool_name?.toLowerCase() ?? '';
  const i = input.tool_input ?? {};

  // 파일 접근 tools
  if (t === 'read' || t === 'write' || t === 'edit' || t === 'glob' || t === 'grep') {
    const fp = (i.file_path ?? i.path ?? '') as string;
    if (fp) {
      const d = checkPath(fp, opts);
      if (!d.allow) return d;
    }
  }

  // Bash command — heuristic으로 시크릿 파일 접근 패턴 차단
  if (t === 'bash' && typeof i.command === 'string') {
    const cmd = i.command;
    // 위험 패턴: 시크릿 파일 직접 cat/echo
    const dangerous = [
      /\$HOME\/\.ssh/,
      /\$HOME\/\.aws/,
      /~\/\.ssh\//,
      /~\/\.aws\//,
      /\bcat\s+.*\.env/,
      /\bcat\s+.*\.pem/,
      /\bcat\s+.*credentials/,
      /\bsudo\b/, // sudo 자체 차단
    ];
    for (const pat of dangerous) {
      if (pat.test(cmd)) {
        return {
          allow: false,
          reason: `위험한 bash 명령 차단: 패턴 ${pat}`,
        };
      }
    }

    // cd or working dir change 후 절대경로 cat 검출 — 한정적이라 정밀 매칭 어려움
  }

  return { allow: true };
}

/**
 * payload에서 시크릿 의심 토큰을 마스킹.
 * inmail/event payload를 외부(UI, MCP)로 노출하기 전에 통과시킴.
 */
export function maskSecrets(text: string): string {
  return text
    // GitHub PAT (ghp_, gho_, ghu_, ghs_, ghr_)
    .replace(/\b(gh[pousr])_[A-Za-z0-9]{36,}/g, '$1_***MASKED***')
    // AWS Access Key
    .replace(/\b(AKIA|ASIA)[0-9A-Z]{16}\b/g, '$1***MASKED***')
    // OpenAI / Anthropic-ish sk-... keys
    .replace(/\bsk-[A-Za-z0-9_\-]{20,}/g, 'sk-***MASKED***')
    // generic Bearer tokens
    .replace(/\b(Bearer\s+)[A-Za-z0-9._\-]{20,}/g, '$1***MASKED***');
}
