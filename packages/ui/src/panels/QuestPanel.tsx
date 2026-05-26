import { Box, Text } from 'ink';
import type { Quest } from '@luida/core';
import { Window } from '../components/Window';
import { QuestRow } from '../components/QuestRow';
import { colors } from '../style/tokens';

export type QuestPanelProps = {
  quests: Quest[];
  focusedIndex: number;
  focused: boolean;
};

export function QuestPanel({
  quests,
  focusedIndex,
  focused,
}: QuestPanelProps): JSX.Element {
  return (
    <Window title="📜 의뢰서" focused={focused}>
      {quests.length === 0 ? (
        <Text color={colors.textDim}>
          오늘은 평화로운 하루입니다. 술집이 조용하네요. 🍺
        </Text>
      ) : (
        <Box flexDirection="column">
          {quests.map((q, i) => (
            <QuestRow
              key={q.id}
              quest={q}
              focused={focused && i === focusedIndex}
            />
          ))}
        </Box>
      )}
    </Window>
  );
}
