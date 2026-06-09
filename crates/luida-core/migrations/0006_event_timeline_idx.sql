-- Luida v2 — events 타임라인 인덱스.
-- 상세 뷰(EventRepo::for_campaign / for_quest)는
--   WHERE campaign_id|quest_id = ? ORDER BY occurred_at DESC, id DESC LIMIT N
-- 로 최신 N건을 가져온다(꼬리추적). 기존엔 campaign_id/quest_id 단독 인덱스가 없어
-- 필터가 풀스캔으로 떨어졌다. 복합 인덱스로 필터 + 정렬을 한 번에 커버한다.
CREATE INDEX IF NOT EXISTS ix_events_campaign ON events(campaign_id, occurred_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS ix_events_quest ON events(quest_id, occurred_at DESC, id DESC);
