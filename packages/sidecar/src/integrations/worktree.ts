import type { Worktree, WorktreeHandle } from '@luida/core';

/**
 * worktrunk CLI(`wt`)를 통한 Worktree 구현.
 *
 * 표준 명령어 `wt c "<name>"`는
 * `wt switch "<name>" --create --base origin/main --execute=claude`의 alias.
 * 내부 호출에서는 `--execute :` (no-op shell)로 override해서
 * worker는 별도 ClaudeWorkerRunner가 띄움 (사용자 메모리 규약 — 외부 노출 명령어는
 * `wt c`로 통일하지만 내부 자동화는 alias 우회 허용).
 *
 * worktree 경로 확인은 `git worktree list --porcelain` 사용 (M2 대응).
 * porcelain 포맷은 안정적이고 한글/공백 경로도 안전.
 */
export class WorktrunkWorktree implements Worktree {
  constructor(
    private readonly bin = 'wt',
    private readonly baseRef = 'origin/main',
  ) {}

  async create(repoPath: string, branchName: string): Promise<WorktreeHandle> {
    const proc = Bun.spawn(
      [
        this.bin,
        'switch',
        branchName,
        '--create',
        '--base',
        this.baseRef,
        '--execute',
        ':',
      ],
      {
        cwd: repoPath,
        stdout: 'pipe',
        stderr: 'pipe',
      },
    );
    const [stdout, stderr] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
    ]);
    const exit = await proc.exited;
    if (exit !== 0) {
      throw new Error(
        `wt switch failed (${exit}): ${stderr.trim() || stdout.trim()}`,
      );
    }

    const path = await this.resolveByGit(repoPath, branchName);
    return { branch: branchName, path };
  }

  /**
   * `git worktree list --porcelain`으로 branch에 매칭되는 worktree path를 찾는다.
   *
   * porcelain 출력 형식 (한 블록):
   *   worktree /abs/path
   *   HEAD <sha>
   *   branch refs/heads/<branch>
   *   (blank line)
   */
  private async resolveByGit(
    repoPath: string,
    branchName: string,
  ): Promise<string> {
    const proc = Bun.spawn(
      ['git', '-C', repoPath, 'worktree', 'list', '--porcelain'],
      { stdout: 'pipe', stderr: 'pipe' },
    );
    const [stdout, stderr] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
    ]);
    const exit = await proc.exited;
    if (exit !== 0) {
      throw new Error(`git worktree list 실패 (${exit}): ${stderr.trim()}`);
    }

    const refPath = `refs/heads/${branchName}`;
    const blocks = stdout.split(/\n\s*\n/);
    for (const block of blocks) {
      if (!block.includes(`branch ${refPath}`)) continue;
      const m = block.match(/^worktree (.+)$/m);
      if (m && m[1]) return m[1].trim();
    }
    throw new Error(`worktree path not found for branch ${branchName}`);
  }
}
