import { Box, Text, useApp, useInput } from 'ink';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactElement } from 'react';
import {
  type Repos,
  createRepos,
  openDb,
} from '@luida/core';
import { type TavernSnapshot, loadSnapshot } from './state/load';
import { AdventurerPanel } from './panels/AdventurerPanel';
import { QuestPanel } from './panels/QuestPanel';
import { TavernLogPanel } from './panels/TavernLogPanel';
import { ChroniclePanel } from './panels/ChroniclePanel';
import { colors } from './style/tokens';

const PANEL_NAMES = ['모험가', '의뢰', '게시판', '연감'] as const;
type PanelIdx = 0 | 1 | 2 | 3;
const PANEL_COUNT = PANEL_NAMES.length;

export type AppProps = {
  dbPath?: string;
  intervalMs?: number;
  /** 외부에서 repos를 주입하면 자체 DB 오픈 안 함 (테스트용) */
  reposOverride?: Repos;
};

/** focusedPanel별 cursor 가능 길이 */
export function panelLength(
  panel: PanelIdx,
  snap: TavernSnapshot,
): number {
  switch (panel) {
    case 0:
      return snap.adventurers.length;
    case 1:
      return snap.recentQuests.length;
    case 2:
      return snap.recentInmail.length;
    case 3:
      return 0;
  }
}

export function App({
  dbPath,
  intervalMs = 1_000,
  reposOverride,
}: AppProps): ReactElement {
  const { exit } = useApp();
  const [snapshot, setSnapshot] = useState<TavernSnapshot | null>(null);
  const [focusedPanel, setFocusedPanel] = useState<PanelIdx>(0);
  const [cursor, setCursor] = useState<Record<PanelIdx, number>>({
    0: 0,
    1: 0,
    2: 0,
    3: 0,
  });
  const [error, setError] = useState<string | null>(null);

  // snapshot ref — useInput 콜백이 항상 최신 snapshot을 보도록
  const snapshotRef = useRef<TavernSnapshot | null>(null);
  snapshotRef.current = snapshot;
  const focusedPanelRef = useRef<PanelIdx>(focusedPanel);
  focusedPanelRef.current = focusedPanel;

  // DB polling
  useEffect(() => {
    let cancelled = false;
    let repos = reposOverride;
    let db: ReturnType<typeof openDb> | null = null;
    if (!repos) {
      try {
        db = openDb(dbPath);
        repos = createRepos(db);
      } catch (err) {
        if (!cancelled) setError(`DB 오픈 실패: ${(err as Error).message}`);
        // cleanup도 항상 반환 (early return 시에도 effect 일관성 유지)
        return () => {
          cancelled = true;
        };
      }
    }

    const tick = (): void => {
      if (cancelled || !repos) return;
      try {
        const snap = loadSnapshot(repos);
        if (cancelled) return;
        setSnapshot(snap);
        setError((prev) => (prev ? null : prev)); // 자동 복구
      } catch (err) {
        if (cancelled) return;
        setError(`로드 실패: ${(err as Error).message}`);
      }
    };
    tick();
    const t = setInterval(tick, intervalMs);
    return () => {
      cancelled = true;
      clearInterval(t);
      if (db && !reposOverride) {
        try {
          repos?.close();
        } catch (err) {
          if (process.env.LUIDA_DEBUG === '1') console.error(err);
        }
        try {
          db.close();
        } catch (err) {
          if (process.env.LUIDA_DEBUG === '1') console.error(err);
        }
      }
    };
  }, [dbPath, intervalMs, reposOverride]);

  const moveCursor = useCallback((delta: number): void => {
    const snap = snapshotRef.current;
    if (!snap) return;
    const panel = focusedPanelRef.current;
    setCursor((prev) => {
      const len = panelLength(panel, snap);
      if (len === 0) return prev;
      const cur = prev[panel];
      return {
        ...prev,
        [panel]: Math.max(0, Math.min(len - 1, cur + delta)),
      };
    });
  }, []);

  useInput((input, key) => {
    if (input === 'q' || (key.ctrl && input === 'c')) {
      exit();
      return;
    }
    if (key.tab) {
      setFocusedPanel((p) => {
        const next = key.shift ? (p + PANEL_COUNT - 1) % PANEL_COUNT : (p + 1) % PANEL_COUNT;
        return next as PanelIdx;
      });
      return;
    }
    if (input === 'j' || key.downArrow) {
      moveCursor(1);
      return;
    }
    if (input === 'k' || key.upArrow) {
      moveCursor(-1);
      return;
    }
  });

  if (error) {
    return (
      <Box>
        <Text color={colors.hpRed}>⚠ {error}</Text>
      </Box>
    );
  }

  if (!snapshot) {
    return (
      <Box>
        <Text color={colors.textDim}>🏮 술집을 여는 중...</Text>
      </Box>
    );
  }

  return (
    <Box flexDirection="column">
      <Box marginBottom={1} gap={2}>
        <Text bold color={colors.textGold}>
          🏮 루이다의 술집
        </Text>
        <Text color={colors.textDim}>
          {PANEL_NAMES[focusedPanel]} 패널 (Tab/Shift+Tab: 전환, j/k: 이동, q: 종료)
        </Text>
      </Box>
      <Box flexDirection="row">
        <Box width="50%" flexDirection="column" flexShrink={1}>
          <AdventurerPanel
            adventurers={snapshot.adventurers}
            activeQuests={snapshot.activeQuests}
            questCountByAdventurer={snapshot.questCountByAdventurer}
            focusedIndex={cursor[0]}
            focused={focusedPanel === 0}
          />
          <TavernLogPanel
            inmail={snapshot.recentInmail}
            focused={focusedPanel === 2}
          />
        </Box>
        <Box width="50%" flexDirection="column" flexShrink={1}>
          <QuestPanel
            quests={snapshot.recentQuests}
            focusedIndex={cursor[1]}
            focused={focusedPanel === 1}
          />
          <ChroniclePanel focused={focusedPanel === 3} />
        </Box>
      </Box>
    </Box>
  );
}
