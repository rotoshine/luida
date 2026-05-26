import { Text } from 'ink';
import { Window } from '../components/Window';
import { colors } from '../style/tokens';

export type ChroniclePanelProps = {
  focused: boolean;
};

export function ChroniclePanel({ focused }: ChroniclePanelProps): JSX.Element {
  return (
    <Window title="📓 연감" focused={focused}>
      <Text color={colors.textDim}>
        Phase 5에서 학습된 패턴이 여기 표시됩니다.
      </Text>
    </Window>
  );
}
