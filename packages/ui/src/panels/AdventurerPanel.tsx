import { Box, Text } from 'ink';
import type { Adventurer, Quest } from '@luida/core';
import { Window } from '../components/Window';
import { AdventurerCard } from '../components/AdventurerCard';
import { colors } from '../style/tokens';
import { deriveStats } from '../util/stats';

export type AdventurerPanelProps = {
  adventurers: Adventurer[];
  activeQuests: Quest[];
  questCountByAdventurer: Map<string, number>;
  focusedIndex: number;
  focused: boolean;
};

export function AdventurerPanel({
  adventurers,
  activeQuests,
  questCountByAdventurer,
  focusedIndex,
  focused,
}: AdventurerPanelProps): JSX.Element {
  const activeByName = new Map<string, Quest>();
  for (const q of activeQuests) {
    if (!activeByName.has(q.dispatched_to)) activeByName.set(q.dispatched_to, q);
  }
  return (
    <Window title="🍺 등록된 모험가" focused={focused}>
      {adventurers.length === 0 ? (
        <Text color={colors.textDim}>
          아직 등록된 모험가가 없습니다. sidecar를 띄워주세요.
        </Text>
      ) : (
        <Box flexDirection="column">
          {adventurers.map((adv, i) => (
            <AdventurerCard
              key={adv.name}
              adventurer={adv}
              stats={deriveStats(
                adv,
                questCountByAdventurer.get(adv.name) ?? 0,
              )}
              activeQuestId={activeByName.get(adv.name)?.id ?? null}
              focused={focused && i === focusedIndex}
            />
          ))}
        </Box>
      )}
    </Window>
  );
}
