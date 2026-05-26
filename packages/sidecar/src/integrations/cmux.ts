import type { CmuxBridge, CmuxTarget } from '@luida/core';

/**
 * 실제 cmux CLI를 호출하는 CmuxBridge.
 *   - `cmux send-key --workspace <ws> --surface <sf> "<text>"`
 *   - `cmux read-screen --workspace <ws> --surface <sf>`
 */
export class CmuxCliBridge implements CmuxBridge {
  constructor(private readonly bin = 'cmux') {}

  async sendPrompt(target: CmuxTarget, text: string): Promise<void> {
    await this.run([
      'send-key',
      '--workspace',
      target.workspace_id,
      '--surface',
      target.surface_id,
      text,
    ]);
    await this.run([
      'send-key',
      '--workspace',
      target.workspace_id,
      '--surface',
      target.surface_id,
      'enter',
    ]);
  }

  async readScreen(target: CmuxTarget): Promise<string> {
    return await this.run([
      'read-screen',
      '--workspace',
      target.workspace_id,
      '--surface',
      target.surface_id,
    ]);
  }

  private async run(args: string[]): Promise<string> {
    const proc = Bun.spawn([this.bin, ...args], {
      stdout: 'pipe',
      stderr: 'pipe',
    });
    const stdout = await new Response(proc.stdout).text();
    const stderr = await new Response(proc.stderr).text();
    const exit = await proc.exited;
    if (exit !== 0) {
      throw new Error(
        `cmux ${args[0]} exited ${exit}: ${stderr.trim() || stdout.trim()}`,
      );
    }
    return stdout;
  }
}
