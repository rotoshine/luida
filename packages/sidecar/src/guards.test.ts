import { describe, expect, test } from 'bun:test';
import {
  DEFAULT_SECRET_PATTERNS,
  checkPath,
  defaultBlockedPaths,
  evaluateHook,
  maskSecrets,
} from './guards';

const opts = {
  worktreeRoot: '/repos/agora/.worktrees/feat-x',
  blockedPaths: ['/Users/roto/.ssh', '/Users/roto/.aws'],
  secretPatterns: DEFAULT_SECRET_PATTERNS,
};

describe('checkPath', () => {
  test('worktree 안 일반 파일은 허용', () => {
    expect(
      checkPath('/repos/agora/.worktrees/feat-x/src/a.ts', opts).allow,
    ).toBe(true);
  });

  test('worktree 밖 절대경로는 차단', () => {
    const d = checkPath('/etc/passwd', opts);
    expect(d.allow).toBe(false);
    expect(d.reason).toContain('worktree 밖');
  });

  test('상대경로도 worktree 밖이면 차단', () => {
    const d = checkPath('../../../../etc/passwd', { ...opts });
    expect(d.allow).toBe(false);
  });

  test('차단된 경로(SSH/AWS) prefix는 차단', () => {
    const d = checkPath('/Users/roto/.ssh/id_rsa', {
      ...opts,
      worktreeRoot: '/Users/roto',
    });
    expect(d.allow).toBe(false);
    expect(d.reason).toContain('차단된 경로');
  });

  test('시크릿 파일명(.env, .pem, credentials.json)은 차단', () => {
    const root = '/repos/agora/.worktrees/feat-x';
    expect(checkPath(`${root}/.env`, { ...opts }).allow).toBe(false);
    expect(checkPath(`${root}/.env.local`, { ...opts }).allow).toBe(false);
    expect(checkPath(`${root}/cert.pem`, { ...opts }).allow).toBe(false);
    expect(checkPath(`${root}/credentials.json`, { ...opts }).allow).toBe(false);
    expect(checkPath(`${root}/src/normal.ts`, { ...opts }).allow).toBe(true);
  });
});

describe('evaluateHook', () => {
  test('Read tool로 worktree 안 → allow', () => {
    const d = evaluateHook(
      {
        tool_name: 'Read',
        tool_input: { file_path: '/repos/agora/.worktrees/feat-x/a.ts' },
      },
      opts,
    );
    expect(d.allow).toBe(true);
  });

  test('Write tool로 worktree 밖 → block', () => {
    const d = evaluateHook(
      {
        tool_name: 'Write',
        tool_input: { file_path: '/Users/roto/some-other-place/x' },
      },
      opts,
    );
    expect(d.allow).toBe(false);
  });

  test('Bash sudo는 차단', () => {
    const d = evaluateHook(
      { tool_name: 'Bash', tool_input: { command: 'sudo rm -rf /' } },
      opts,
    );
    expect(d.allow).toBe(false);
  });

  test('Bash cat ~/.ssh/id_rsa 차단', () => {
    const d = evaluateHook(
      {
        tool_name: 'Bash',
        tool_input: { command: 'cat ~/.ssh/id_rsa' },
      },
      opts,
    );
    expect(d.allow).toBe(false);
  });

  test('Bash 일반 명령 허용', () => {
    const d = evaluateHook(
      { tool_name: 'Bash', tool_input: { command: 'ls -la' } },
      opts,
    );
    expect(d.allow).toBe(true);
  });

  test('알 수 없는 tool은 기본 허용', () => {
    expect(evaluateHook({ tool_name: 'UnknownTool' }, opts).allow).toBe(true);
  });
});

describe('maskSecrets', () => {
  test('GitHub PAT 마스킹', () => {
    expect(
      maskSecrets('token=ghp_aaaabbbbccccddddeeeeffffggggh4444jjjjkkkk'),
    ).toContain('ghp_***MASKED***');
  });

  test('AWS access key 마스킹', () => {
    expect(maskSecrets('AKIAIOSFODNN7EXAMPLE')).toContain('AKIA***MASKED***');
  });

  test('Bearer token 마스킹', () => {
    expect(maskSecrets('Authorization: Bearer abcdefghijklmnopqrstuvwxyz12')).toContain(
      'Bearer ***MASKED***',
    );
  });

  test('sk- 토큰 마스킹', () => {
    expect(maskSecrets('OPENAI_KEY=sk-abcdefghijklmnopqrstuvwxyz1234')).toContain(
      'sk-***MASKED***',
    );
  });

  test('정상 텍스트는 그대로', () => {
    expect(maskSecrets('hello world 안녕')).toBe('hello world 안녕');
  });
});

describe('defaultBlockedPaths', () => {
  test('$HOME 기반 경로 반환', () => {
    const paths = defaultBlockedPaths();
    expect(paths.some((p) => p.includes('.ssh'))).toBe(true);
    expect(paths.some((p) => p.includes('.aws'))).toBe(true);
  });
});
