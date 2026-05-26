import { Box, Text } from 'ink';
import type { Adventurer } from '@luida/core';
import { colors } from '../style/tokens';
import { type AdventurerStats } from '../util/stats';
import { HpBar } from './HpBar';

export type AdventurerCardProps = {
  adventurer: Adventurer;
  stats: AdventurerStats;
  activeQuestId?: number | null;
  focused?: boolean;
};

const STATUS_ICON: Record<string, string> = {
  idle: '🏠',
  busy: '⚔',
  offline: '💤',
};

export function AdventurerCard({
  adventurer,
  stats,
  activeQuestId,
  focused = false,
}: AdventurerCardProps): JSX.Element {
  const icon = STATUS_ICON[adventurer.status] ?? '·';
  return (
    <Box flexDirection="row" gap={1}>
      <Text color={focused ? colors.textGold : colors.textPrimary}>
        {focused ? '▶' : ' '}
      </Text>
      <Box width={10}>
        <Text bold color={colors.textPrimary}>
          {adventurer.name}
        </Text>
      </Box>
      <Box width={10}>
        <Text color={colors.textDim}>{stats.className}</Text>
      </Box>
      <Text color={colors.textGold}>Lv.{String(stats.level).padStart(2, '0')}</Text>
      <Text>HP </Text>
      <HpBar current={stats.hp.current} max={stats.hp.max} />
      <Text> {icon} </Text>
      <Text color={colors.textDim}>
        {activeQuestId ? `Quest #${activeQuestId}` : adventurer.status}
      </Text>
    </Box>
  );
}
