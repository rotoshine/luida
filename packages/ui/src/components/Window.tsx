import { Box, Text } from 'ink';
import type { ReactNode } from 'react';
import { colors } from '../style/tokens';

export type WindowProps = {
  title?: string;
  children: ReactNode;
  /** 셀 단위 너비 (대략) */
  width?: number | string;
  height?: number | string;
  borderColor?: string;
  focused?: boolean;
};

/** DQ 풍의 흰 더블 라인 박스 (Ink는 single border만 지원하므로 색으로 강조) */
export function Window({
  title,
  children,
  width,
  height,
  borderColor,
  focused = false,
}: WindowProps): JSX.Element {
  return (
    <Box
      flexDirection="column"
      borderStyle="round"
      borderColor={borderColor ?? (focused ? colors.textGold : colors.windowBorder)}
      width={width}
      height={height}
      paddingX={1}
    >
      {title ? (
        <Box marginBottom={1}>
          <Text bold color={focused ? colors.textGold : colors.textPrimary}>
            {focused ? '▶ ' : '  '}
            {title}
          </Text>
        </Box>
      ) : null}
      <Box flexDirection="column" flexGrow={1}>
        {children}
      </Box>
    </Box>
  );
}
