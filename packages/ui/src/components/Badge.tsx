import { Text } from 'ink';
import { type StatusTone, statusColor } from '../style/tokens';

export type BadgeProps = {
  status: StatusTone;
  label?: string;
};

const LABELS: Record<StatusTone, string> = {
  pending: '대기',
  running: '진행',
  reviewing: '검토',
  needs_approval: '승인필요',
  pr_ready: 'PR',
  completed: '완료',
  failed: '실패',
  aborted: '중단',
};

export function Badge({ status, label }: BadgeProps): JSX.Element {
  return (
    <Text color={statusColor(status)} inverse>
      {' '}
      {label ?? LABELS[status]}{' '}
    </Text>
  );
}
