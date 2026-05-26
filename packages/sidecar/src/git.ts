// Worktree 안의 git 작업 헬퍼.
// Phase 3에서는 변경 파일 목록만 필요.

export type GitChangeOpts = {
  cwd: string;
  /** 비교 기준 ref. 기본 origin/main */
  baseRef?: string;
};

/**
 * `git diff --name-only <base>...HEAD`로 변경 파일 목록을 가져온다.
 * worktree에서 worker가 작업한 결과를 평가하기 위함.
 *
 * base...HEAD 형태로 머지 베이스 기준 차이를 봄 (브랜치 단독 변경분).
 */
export async function changedFiles(opts: GitChangeOpts): Promise<string[]> {
  const base = opts.baseRef ?? 'origin/main';
  const proc = Bun.spawn(
    ['git', '-C', opts.cwd, 'diff', '--name-only', `${base}...HEAD`],
    { stdout: 'pipe', stderr: 'pipe' },
  );
  const [stdout, stderr] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
  ]);
  const exit = await proc.exited;
  if (exit !== 0) {
    throw new Error(`git diff 실패 (${exit}): ${stderr.trim()}`);
  }
  return stdout
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0);
}
