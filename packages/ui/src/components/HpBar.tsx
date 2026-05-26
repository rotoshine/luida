import { Text } from 'ink';
import { hpColor } from '../style/tokens';

export type HpBarProps = {
  current: number;
  max: number;
  width?: number;
};

export function HpBar({ current, max, width = 8 }: HpBarProps): JSX.Element {
  const ratio = Math.max(0, Math.min(1, current / Math.max(1, max)));
  const filled = Math.round(ratio * width);
  const empty = width - filled;
  return (
    <Text color={hpColor(ratio)}>
      {'█'.repeat(filled)}
      {'░'.repeat(empty)}
    </Text>
  );
}
