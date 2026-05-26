// @ts-nocheck
// 라이브 데이터 — Bun.serve의 /api/snapshot + /api/stream(SSE)에 연결.
// LiveProvider가 context로 최신 snapshot을 공급, useLive() 훅으로 자식이 구독.

import React, { createContext, useContext, useEffect, useState } from 'react';

export type Adventurer = {
  name: string;
  workspace_id: string;
  surface_id: string;
  repo_path: string | null;
  role: 'main' | 'worker' | 'brain';
  status: 'idle' | 'busy' | 'offline';
  last_seen: number;
  registered_at: number;
};

export type Quest = {
  id: number;
  dispatched_by: string;
  dispatched_to: string;
  brief: string;
  status: string;
  branch: string | null;
  pr_url: string | null;
  progress: string | null;
  updated_at: number;
};

export type Inmail = {
  id: number;
  from_session: string;
  to_session: string;
  kind: string;
  payload: string;
  created_at: number;
};

export type TavernSnapshot = {
  adventurers: Adventurer[];
  quests: Quest[];
  inmail: Inmail[];
  taken_at: number;
};

const LiveContext = createContext<{
  snapshot: TavernSnapshot | null;
  error: string | null;
}>({ snapshot: null, error: null });

export function LiveProvider({ children }: { children: React.ReactNode }) {
  const [snapshot, setSnapshot] = useState<TavernSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    // 1) 초기 스냅샷
    fetch('/api/snapshot')
      .then((r) => r.json())
      .then((data) => {
        if (!cancelled) setSnapshot(data);
      })
      .catch((err) => {
        if (!cancelled) setError(String(err));
      });

    // 2) SSE 라이브 갱신
    const es = new EventSource('/api/stream');
    es.onmessage = (e) => {
      if (cancelled) return;
      try {
        const data = JSON.parse(e.data);
        setSnapshot(data);
        setError(null);
      } catch (err) {
        setError(String(err));
      }
    };
    es.onerror = () => {
      // EventSource는 자동 재연결 — 단순 표시만
      if (!cancelled) setError('SSE 연결 끊김 (자동 재시도 중)');
    };

    return () => {
      cancelled = true;
      es.close();
    };
  }, []);

  return (
    <LiveContext.Provider value={{ snapshot, error }}>
      {children}
    </LiveContext.Provider>
  );
}

export function useLive() {
  return useContext(LiveContext);
}
