import { Box, Text } from 'ink';
import type { Inmail } from '@luida/core';
import { Window } from '../components/Window';
import { EventLogLine } from '../components/EventLogLine';
import { colors } from '../style/tokens';

export type TavernLogPanelProps = {
  inmail: Inmail[];
  focused: boolean;
  limit?: number;
};

export function TavernLogPanel({
  inmail,
  focused,
  limit = 10,
}: TavernLogPanelProps): JSX.Element {
  const lines = inmail.slice(0, limit);
  return (
    <Window title="📰 술집 게시판" focused={focused}>
      {lines.length === 0 ? (
        <Text color={colors.textDim}>아직 소식이 없습니다.</Text>
      ) : (
        <Box flexDirection="column">
          {lines.map((m) => (
            <EventLogLine key={m.id} inmail={m} />
          ))}
        </Box>
      )}
    </Window>
  );
}
