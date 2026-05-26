import type { Inmail } from '@luida/core';

/** inmail 1건을 cmux로 주입할 prompt 텍스트로 렌더 */
export function renderInmailPrompt(msg: Inmail): string {
  const lines = [
    `📬 inmail #${msg.id} from ${msg.from_session} (kind=${msg.kind})`,
  ];
  switch (msg.kind) {
    case 'dispatch': {
      const payload = safeParse(msg.payload);
      lines.push('');
      lines.push('## 새 의뢰 (dispatched)');
      if (typeof payload === 'object' && payload !== null) {
        const p = payload as Record<string, unknown>;
        if (typeof p.brief === 'string') {
          lines.push('');
          lines.push('### Brief');
          lines.push(p.brief);
        }
        if (typeof p.branch === 'string') {
          lines.push('');
          lines.push(`Branch: \`${p.branch}\``);
        }
      } else {
        lines.push(String(payload));
      }
      lines.push('');
      lines.push('이 의뢰를 처리해주세요. 완료 후 한 줄 요약을 남겨주세요.');
      break;
    }
    case 'ack':
    case 'progress':
    case 'alert':
    case 'info':
    case 'proposal':
    default: {
      const payload = safeParse(msg.payload);
      lines.push('');
      lines.push('```json');
      lines.push(JSON.stringify(payload, null, 2));
      lines.push('```');
      break;
    }
  }
  return lines.join('\n');
}

function safeParse(raw: string): unknown {
  try {
    return JSON.parse(raw);
  } catch {
    return raw;
  }
}
