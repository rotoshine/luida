import type { PullRequestOptions, VcsHost } from '@luida/core';

const PR_URL_RE = /https:\/\/github\.com\/[\w.-]+\/[\w.-]+\/pull\/\d+/;

/**
 * `gh pr create`를 호출하는 VcsHost.
 *
 * 견고성 (Phase 1 리뷰 M1):
 *  - head 명시 (worktree branch가 detached 등으로 추측 실패하는 것 방지)
 *  - stdout+stderr 모두에서 PR URL 정규식 추출
 *  - 이미 존재하는 PR이면 gh가 stderr에 알려주므로 같은 정규식으로 흡수
 */
export class GhVcsHost implements VcsHost {
  constructor(private readonly bin = 'gh') {}

  async createPullRequest(opts: PullRequestOptions): Promise<{ url: string }> {
    const args = [
      'pr',
      'create',
      '--title',
      opts.title,
      '--body',
      opts.body,
    ];
    if (opts.base) args.push('--base', opts.base);
    if (opts.head) args.push('--head', opts.head);

    const proc = Bun.spawn([this.bin, ...args], {
      cwd: opts.cwd,
      stdout: 'pipe',
      stderr: 'pipe',
    });
    const [stdout, stderr] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
    ]);
    const exit = await proc.exited;

    const combined = `${stdout}\n${stderr}`;
    const match = combined.match(PR_URL_RE);

    if (match) {
      return { url: match[0] };
    }

    if (exit !== 0) {
      throw new Error(
        `gh pr create failed (${exit}): ${stderr.trim() || stdout.trim()}`,
      );
    }
    throw new Error(`gh pr create did not return URL. stdout=${stdout}`);
  }
}
