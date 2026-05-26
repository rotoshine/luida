// 간단한 명령 라우터. Phase 1 prereq.
//   - "공백 join" 키로 sub-command 매칭 (예: 'db init', 'sidecar')
//   - --key=value, --flag, -k value 형태 지원
//   - 가장 깊게 매칭되는 핸들러를 선택

export type Handler = (ctx: CommandContext) => Promise<void> | void;

export type CommandContext = {
  args: string[]; // 핸들러 키 이후의 positional
  options: Record<string, string | boolean>;
  raw: string[];
};

export type Command = {
  /** 공백으로 구분된 sub-command 키. 예: 'db init', 'sidecar' */
  key: string;
  /** 한 줄 설명 */
  desc: string;
  handler: Handler;
};

export class Router {
  private readonly commands = new Map<string, Command>();

  register(cmd: Command): this {
    this.commands.set(cmd.key, cmd);
    return this;
  }

  list(): Command[] {
    return [...this.commands.values()].sort((a, b) =>
      a.key.localeCompare(b.key),
    );
  }

  /** 가장 긴 prefix 매치를 찾는다. 매치 없으면 null */
  resolve(argv: string[]): { command: Command; ctx: CommandContext } | null {
    const positionals: string[] = [];
    const options: Record<string, string | boolean> = {};
    for (let i = 0; i < argv.length; i++) {
      const token = argv[i]!;
      if (token.startsWith('--')) {
        const eq = token.indexOf('=');
        if (eq >= 0) {
          options[token.slice(2, eq)] = token.slice(eq + 1);
        } else {
          const key = token.slice(2);
          const next = argv[i + 1];
          if (next != null && !next.startsWith('-')) {
            options[key] = next;
            i++;
          } else {
            options[key] = true;
          }
        }
      } else if (token.startsWith('-') && token.length === 2) {
        const key = token.slice(1);
        const next = argv[i + 1];
        if (next != null && !next.startsWith('-')) {
          options[key] = next;
          i++;
        } else {
          options[key] = true;
        }
      } else {
        positionals.push(token);
      }
    }

    // 가장 긴 매칭 키부터 검사
    for (let depth = positionals.length; depth >= 1; depth--) {
      const key = positionals.slice(0, depth).join(' ');
      const command = this.commands.get(key);
      if (command) {
        return {
          command,
          ctx: {
            args: positionals.slice(depth),
            options,
            raw: argv,
          },
        };
      }
    }
    return null;
  }

  formatHelp(): string {
    const lines = ['Usage: luida <command> [options]', '', 'Commands:'];
    const maxKey = Math.max(...this.list().map((c) => c.key.length));
    for (const c of this.list()) {
      lines.push(`  ${c.key.padEnd(maxKey + 2)}${c.desc}`);
    }
    lines.push('');
    lines.push('Options:');
    lines.push('  --help, -h            도움말 표시');
    return lines.join('\n');
  }
}
