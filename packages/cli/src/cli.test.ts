import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const CLI_PATH = join(import.meta.dir, 'index.ts');

let tempDir: string;
let dbPath: string;

beforeEach(async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'luida-cli-test-'));
  dbPath = join(tempDir, 'tavern.db');
});

afterEach(async () => {
  await rm(tempDir, { recursive: true, force: true });
});

async function runCli(
  args: string[],
  env: Record<string, string> = {},
): Promise<{ exitCode: number; stdout: string; stderr: string }> {
  const proc = Bun.spawn(['bun', 'run', CLI_PATH, ...args], {
    env: { ...process.env, LUIDA_DB_PATH: dbPath, ...env },
    stdout: 'pipe',
    stderr: 'pipe',
  });
  const [stdout, stderr] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
  ]);
  const exitCode = await proc.exited;
  return { exitCode, stdout, stderr };
}

describe('luida CLI', () => {
  test('--help prints usage and exits 0', async () => {
    const r = await runCli(['--help']);
    expect(r.exitCode).toBe(0);
    expect(r.stdout).toContain('Usage:');
    expect(r.stdout).toContain('db init');
  });

  test('no args prints usage and exits 0', async () => {
    const r = await runCli([]);
    expect(r.exitCode).toBe(0);
    expect(r.stdout).toContain('Usage:');
  });

  test('unknown command exits 1 with error message', async () => {
    const r = await runCli(['bogus']);
    expect(r.exitCode).toBe(1);
    expect(r.stderr).toContain('알 수 없는 명령');
  });

  test('db init creates DB at LUIDA_DB_PATH', async () => {
    const r = await runCli(['db', 'init']);
    expect(r.exitCode).toBe(0);
    expect(r.stdout).toContain('루이다의 술집');
    expect(r.stdout).toContain('0001_init.sql');
    expect(existsSync(dbPath)).toBe(true);
  });

  test('db init is idempotent — second run reports already-up-to-date', async () => {
    const r1 = await runCli(['db', 'init']);
    expect(r1.exitCode).toBe(0);
    const r2 = await runCli(['db', 'init']);
    expect(r2.exitCode).toBe(0);
    expect(r2.stdout).toContain('이미 최신');
  });

  test('db init creates parent directory if missing', async () => {
    const nested = join(tempDir, 'deep', 'nested', 'tavern.db');
    const r = await runCli(['db', 'init'], { LUIDA_DB_PATH: nested });
    expect(r.exitCode).toBe(0);
    expect(existsSync(nested)).toBe(true);
  });
});
