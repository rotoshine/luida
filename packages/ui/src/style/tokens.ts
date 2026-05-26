// DQ3 풍 색 토큰. Ink는 hex 또는 chalk 색명을 지원.
export const colors = {
  windowBg: '#08197B',
  windowBorder: '#FFFFFF',
  textPrimary: '#FFFFFF',
  textDim: '#A8B8E8',
  textGold: '#FCD34D',
  hpGreen: '#4ADE80',
  hpYellow: '#FACC15',
  hpRed: '#EF4444',
  mpBlue: '#60A5FA',
  accentPink: '#F472B6',
  bg: '#000814',
} as const;

export type StatusTone =
  | 'pending'
  | 'running'
  | 'reviewing'
  | 'needs_approval'
  | 'pr_ready'
  | 'completed'
  | 'failed'
  | 'aborted';

export function statusColor(s: StatusTone): string {
  switch (s) {
    case 'pending':
      return colors.textDim;
    case 'running':
      return colors.mpBlue;
    case 'reviewing':
      return colors.textGold;
    case 'needs_approval':
      return colors.accentPink;
    case 'pr_ready':
      return colors.textGold;
    case 'completed':
      return colors.hpGreen;
    case 'failed':
      return colors.hpRed;
    case 'aborted':
      return colors.hpRed;
  }
}

export function hpColor(ratio: number): string {
  if (ratio < 0.25) return colors.hpRed;
  if (ratio < 0.6) return colors.hpYellow;
  return colors.hpGreen;
}
