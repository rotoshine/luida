import { Box, Text } from 'ink';
import { memo, useMemo } from 'react';
import type { Inmail } from '@luida/core';
import { colors } from '../style/tokens';
import { firstLine, relativeTime } from '../util/stats';

export type EventLogLineProps = {
  inmail: Inmail;
};

const KIND_ICON: Record<string, string> = {
  dispatch: '📜',
  ack: '🍺',
  progress: '⏳',
  proposal: '💡',
  alert: '⚠',
  info: '✉',
};

const KIND_COLOR: Record<string, string> = {
  dispatch: '#FCD34D',
  ack: '#4ADE80',
  progress: '#60A5FA',
  proposal: '#F472B6',
  alert: '#EF4444',
  info: '#A8B8E8',
};

export function extractSummary(payload: string): string {
  try {
    const p = JSON.parse(payload) as Record<string, unknown>;
    if (typeof p.brief === 'string') return p.brief;
    if (typeof p.summary === 'string') return p.summary;
    if (typeof p.msg === 'string') return p.msg;
    if (typeof p.question === 'string') return p.question;
    return JSON.stringify(p).slice(0, 60);
  } catch {
    return payload.slice(0, 60);
  }
}

function EventLogLineImpl({ inmail }: EventLogLineProps): JSX.Element {
  const icon = KIND_ICON[inmail.kind] ?? '·';
  const color = KIND_COLOR[inmail.kind] ?? colors.textPrimary;
  const summary = useMemo(
    () => extractSummary(inmail.payload),
    [inmail.payload],
  );
  return (
    <Box flexDirection="row" gap={1}>
      <Text color={colors.textDim}>{relativeTime(inmail.created_at)}</Text>
      <Text color={color}>
        {icon} {inmail.from_session} → {inmail.to_session}
      </Text>
      <Text color={colors.textPrimary}>{firstLine(summary, 60)}</Text>
    </Box>
  );
}

export const EventLogLine = memo(EventLogLineImpl);
