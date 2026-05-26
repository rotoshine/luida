import { Box, Text } from 'ink';
import type { Quest } from '@luida/core';
import { colors } from '../style/tokens';
import { firstLine, questProgressRatio } from '../util/stats';
import { Badge } from './Badge';

export type QuestRowProps = {
  quest: Quest;
  focused?: boolean;
};

export function QuestRow({ quest, focused = false }: QuestRowProps): JSX.Element {
  const ratio = questProgressRatio(quest);
  const width = 8;
  const filled = Math.round(ratio * width);
  const bar = '▓'.repeat(filled) + '░'.repeat(width - filled);

  return (
    <Box flexDirection="row" gap={1}>
      <Text color={focused ? colors.textGold : colors.textPrimary}>
        {focused ? '▶' : ' '}
      </Text>
      <Text color={colors.textDim}>#{String(quest.id).padStart(3, ' ')}</Text>
      <Box width={10}>
        <Text color={colors.textPrimary}>{quest.dispatched_to}</Text>
      </Box>
      <Box flexGrow={1}>
        <Text color={colors.textPrimary}>{firstLine(quest.brief, 40)}</Text>
      </Box>
      <Text color={colors.mpBlue}>{bar}</Text>
      <Badge status={quest.status} />
    </Box>
  );
}
