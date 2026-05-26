import {
  type Quest,
  type Relationship,
  type RelationshipRepo,
  type Repos,
  type YamlRelationship,
  pathMatchesAny,
  parseRelationshipsYaml,
} from '@luida/core';

export type PostQuestContext = {
  quest: Quest;
  /** path_changed 룰을 평가하기 위한 변경 파일 목록 (worker가 만든 diff) */
  changedFiles?: string[];
};

export type FollowUp = {
  rule: Relationship;
  /** dispatch 또는 proposal payload에 들어갈 brief */
  brief: string;
};

export type EvaluateResult = {
  autoDispatch: FollowUp[];
  proposals: FollowUp[];
};

/**
 * 완료된 quest에 대해 enabled relationships를 평가한다.
 * 매칭된 룰을 action별로 분류해 반환.
 */
export function evaluatePostQuest(
  repos: Repos,
  ctx: PostQuestContext,
): EvaluateResult {
  const allEnabled = repos.relationships.listEnabled();
  const fromSession = ctx.quest.dispatched_to; // 완료한 모험가가 다음 트리거의 from
  const relevant = allEnabled.filter((r) => r.from_session === fromSession);

  const autoDispatch: FollowUp[] = [];
  const proposals: FollowUp[] = [];

  for (const rel of relevant) {
    let triggerConfig: unknown;
    try {
      triggerConfig = JSON.parse(rel.trigger_config);
    } catch {
      continue;
    }

    let matchedFiles: string[] | null = null;
    if (rel.trigger_kind === 'path_changed') {
      const paths = (triggerConfig as { paths?: unknown })?.paths;
      if (!Array.isArray(paths)) continue;
      const patterns = paths.filter((p): p is string => typeof p === 'string');
      const changed = ctx.changedFiles ?? [];
      matchedFiles = changed.filter((f) => pathMatchesAny(f, patterns));
      if (matchedFiles.length === 0) continue;
    } else if (rel.trigger_kind === 'quest_completed') {
      const requiredStatus = (triggerConfig as { status?: unknown })?.status;
      if (
        typeof requiredStatus === 'string' &&
        ctx.quest.status !== requiredStatus
      ) {
        continue;
      }
    } else if (rel.trigger_kind === 'tag_pushed') {
      // Phase 3에서는 지원만 선언, 매칭은 false (Phase 4에서 본격화)
      continue;
    }

    const brief = renderTemplate(rel.brief_template, {
      files: matchedFiles?.join(', ') ?? '',
      quest_id: String(ctx.quest.id),
      from: ctx.quest.dispatched_by,
      to: rel.to_session,
      brief: ctx.quest.brief,
    });

    const item: FollowUp = { rule: rel, brief };
    if (rel.action === 'auto_dispatch') {
      autoDispatch.push(item);
    } else {
      proposals.push(item);
    }
  }

  return { autoDispatch, proposals };
}

/**
 * 평가 결과를 inmail로 실행한다.
 *  - auto_dispatch: target에게 dispatch kind inmail
 *  - propose: 원본 dispatcher에게 proposal kind inmail
 *
 * 멱등성: dedupe_key = `chain:quest-<src>-rel-<relId>` 형태
 */
export function applyFollowUps(
  repos: Repos,
  from: string,
  ctx: PostQuestContext,
  result: EvaluateResult,
): { dispatched: number; proposed: number } {
  let dispatched = 0;
  let proposed = 0;

  // dedupe_key는 rel.name 우선 (yaml 재싱크 시 rel.id가 바뀌어도 안정)
  // name이 없으면 id로 fallback
  const stableKey = (f: FollowUp): string =>
    f.rule.name ?? `id${f.rule.id}`;

  for (const f of result.autoDispatch) {
    const r = repos.inmail.enqueue({
      from_session: from,
      to_session: f.rule.to_session,
      kind: 'dispatch',
      payload: {
        brief: f.brief,
        from_chain: { quest_id: ctx.quest.id, rule: f.rule.name ?? null },
      },
      quest_id: ctx.quest.id,
      dedupe_key: `chain:quest-${ctx.quest.id}-rule-${stableKey(f)}`,
    });
    if (r.inserted) dispatched += 1;
  }

  for (const f of result.proposals) {
    const r = repos.inmail.enqueue({
      from_session: from,
      to_session: ctx.quest.dispatched_by,
      kind: 'proposal',
      payload: {
        rule_name: f.rule.name,
        brief: f.brief,
        question: `${f.rule.name ?? '룰'} 자동화 실행 승인?`,
      },
      quest_id: ctx.quest.id,
      dedupe_key: `proposal:quest-${ctx.quest.id}-rule-${stableKey(f)}`,
    });
    if (r.inserted) proposed += 1;
  }

  return { dispatched, proposed };
}

function renderTemplate(
  template: string | null | undefined,
  vars: Record<string, string>,
): string {
  if (!template) {
    return `[chain] ${vars.from ?? ''} → ${vars.to ?? ''}: ${vars.brief ?? ''}`;
  }
  return template.replace(/\{(\w+)\}/g, (_m, key: string) => vars[key] ?? `{${key}}`);
}

/**
 * yaml 파일에서 룰을 읽어 DB에 동기화한다.
 *  - 같은 name이면 update (현재 구현: 단순 INSERT — name UNIQUE 충돌은 무시)
 *  - 향후 Phase 4에서 upsert 패턴으로 개선
 */
/**
 * yaml 텍스트의 룰을 DB에 동기화한다. name 기준 upsert.
 * - name이 있으면: 기존 row update (yaml SOT 보장)
 * - name이 없으면: 단순 insert (중복 방지 책임은 yaml 작성자)
 */
export function syncRelationshipsFromYaml(
  repoFacade: RelationshipRepo,
  yamlText: string,
  source: 'human' | 'learned-promoted' = 'human',
): { added: number; updated: number; failed: number } {
  const yamlRels = parseRelationshipsYaml(yamlText);
  let added = 0;
  let updated = 0;
  let failed = 0;
  for (const y of yamlRels) {
    try {
      const r = repoFacade.upsertByName({
        name: y.name ?? null,
        from_session: y.from,
        trigger_kind: y.trigger.kind,
        trigger_config: serializeTrigger(y.trigger),
        to_session: y.to,
        action: y.action,
        brief_template: y.brief_template ?? null,
        enabled: y.enabled === false ? 0 : 1,
        source,
        confidence: null,
      });
      if (r.created) added += 1;
      else updated += 1;
    } catch (err) {
      console.warn(
        `[brain] syncRelationships: rule "${y.name ?? '<unnamed>'}" 실패:`,
        (err as Error).message,
      );
      failed += 1;
    }
  }
  return { added, updated, failed };
}

function serializeTrigger(t: YamlRelationship['trigger']): unknown {
  if (t.kind === 'path_changed') return { paths: t.paths };
  if (t.kind === 'quest_completed') return { status: t.status };
  if (t.kind === 'tag_pushed') return { pattern: t.pattern };
  return {};
}
