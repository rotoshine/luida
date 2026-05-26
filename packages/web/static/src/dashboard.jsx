// Main dashboard layout — 4-panel tavern + quest detail modal + keyboard nav.

const { useState: useState_D, useEffect: useEffect_D, useMemo: useMemo_D, useRef: useRef_D } = React;

function DashboardTab({ theme, lang, density, onShowToast }) {
  const data = window.LUIDA_DATA;
  const i18n = window.LUIDA_I18N[lang];
  const [filter, setFilter] = useState_D('all');
  const [questId, setQuestId] = useState_D(null);
  const [advName, setAdvName] = useState_D(data.adventurers[0].name);
  const [focus, setFocus] = useState_D('quests'); // which panel keyboard targets
  const [cmdOpen, setCmdOpen] = useState_D(false);

  const filteredQuests = useMemo_D(() => {
    if (filter === 'all') return data.quests;
    if (filter === 'active') return data.quests.filter((q) => ['pending', 'running', 'reviewing'].includes(q.status));
    if (filter === 'needs_approval') return data.quests.filter((q) => q.status === 'needs_approval');
    if (filter === 'done') return data.quests.filter((q) => ['completed', 'pr_ready'].includes(q.status));
    if (filter === 'failed') return data.quests.filter((q) => ['failed', 'aborted'].includes(q.status));
    return data.quests;
  }, [filter, data.quests]);

  const [questCursor, setQuestCursor] = useState_D(0);
  useEffect_D(() => setQuestCursor(0), [filter]);

  // Keyboard: j/k move within active panel, Enter opens, gq/ga switch focus, ⌘K palette
  const lastKey = useRef_D('');
  useEffect_D(() => {
    const onKey = (e) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        setCmdOpen((v) => !v);
        return;
      }
      if (cmdOpen) {
        if (e.key === 'Escape') setCmdOpen(false);
        return;
      }
      if (questId !== null) {
        if (e.key === 'Escape') setQuestId(null);
        return;
      }
      const k = e.key;
      if (k === 'g') { lastKey.current = 'g'; return; }
      if (lastKey.current === 'g') {
        lastKey.current = '';
        if (k === 'q') setFocus('quests');
        else if (k === 'a') setFocus('adventurers');
        else if (k === 'l') setFocus('tavern');
        else if (k === 'c') setFocus('chronicle');
        return;
      }
      if (k === 'j' || k === 'ArrowDown') {
        e.preventDefault();
        if (focus === 'quests') setQuestCursor((c) => Math.min(filteredQuests.length - 1, c + 1));
        else if (focus === 'adventurers') {
          const i = data.adventurers.findIndex((a) => a.name === advName);
          setAdvName(data.adventurers[Math.min(data.adventurers.length - 1, i + 1)].name);
        }
      } else if (k === 'k' || k === 'ArrowUp') {
        e.preventDefault();
        if (focus === 'quests') setQuestCursor((c) => Math.max(0, c - 1));
        else if (focus === 'adventurers') {
          const i = data.adventurers.findIndex((a) => a.name === advName);
          setAdvName(data.adventurers[Math.max(0, i - 1)].name);
        }
      } else if (k === 'Enter') {
        if (focus === 'quests' && filteredQuests[questCursor]) {
          setQuestId(filteredQuests[questCursor].id);
        }
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [focus, advName, questCursor, filteredQuests, questId, cmdOpen]);

  const selectedQuest = data.quests.find((q) => q.id === questId);
  const selectedAdv = data.adventurers.find((a) => a.name === advName);

  const pad = density === 'compact' ? 8 : density === 'comfy' ? 16 : 12;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      {/* Header strip */}
      <div
        style={{
          background: theme.win,
          border: `2px solid ${theme.border}`,
          padding: 3,
        }}
      >
        <div
          style={{
            border: `2px solid ${theme.border}`,
            padding: '10px 14px',
            display: 'flex',
            alignItems: 'center',
            gap: 16,
          }}
        >
          <div
            style={{
              fontFamily: 'Galmuri11, "DotGothic16", monospace',
              fontSize: 18,
              color: theme.gold,
              letterSpacing: 1,
              display: 'flex',
              alignItems: 'center',
              gap: 8,
            }}
          >
            <span>🍺</span>
            <span>{lang === 'ko' ? '루이다의 주점' : "LUIDA'S TAVERN"}</span>
          </div>
          <div
            style={{
              fontFamily: 'Galmuri11, "DotGothic16", monospace',
              fontSize: 12,
              color: theme.dim,
              flex: 1,
            }}
          >
            {i18n.welcome}
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 14, fontFamily: '"DotGothic16", monospace', fontSize: 11, color: theme.dim, letterSpacing: 1 }}>
            <span>
              <span style={{ color: theme.green }}>●</span> SSE LIVE
            </span>
            <span>
              <span style={{ color: theme.gold }}>QUESTS</span> {data.quests.filter((q) => ['pending', 'running', 'reviewing'].includes(q.status)).length}/{data.quests.length}
            </span>
            <span>
              <span style={{ color: theme.pink }}>NEEDS_APPROVAL</span> {data.quests.filter((q) => q.status === 'needs_approval').length}
            </span>
            <span style={{ color: theme.dim }}>
              ⌘K
            </span>
          </div>
        </div>
      </div>

      {/* 4-panel grid: adventurers | quests / tavern | chronicle */}
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: '340px 1fr',
          gridTemplateRows: 'auto auto',
          gap: 14,
        }}
      >
        {/* Adventurer panel — left column, spans both rows */}
        <div style={{ gridRow: '1 / span 2' }}>
          <Window theme={theme} title={i18n.tabs.adventurers} accent={focus === 'adventurers' ? '◀ ACTIVE' : 'g a'}>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              {data.adventurers.map((adv) => (
                <AdventurerCard
                  key={adv.name}
                  theme={theme}
                  adv={adv}
                  lang={lang}
                  selected={adv.name === advName && focus === 'adventurers'}
                  onClick={() => { setAdvName(adv.name); setFocus('adventurers'); }}
                />
              ))}
            </div>
          </Window>
        </div>

        {/* Quest board */}
        <div>
          <Window
            theme={theme}
            title={i18n.tabs.quests}
            accent={focus === 'quests' ? '◀ ACTIVE · j/k · ENTER' : 'g q'}
          >
            <div style={{ display: 'flex', gap: 6, marginBottom: 8, flexWrap: 'wrap' }}>
              {[
                ['all', i18n.filters[0]],
                ['active', i18n.filters[1]],
                ['needs_approval', i18n.filters[2]],
                ['done', i18n.filters[3]],
                ['failed', i18n.filters[4]],
              ].map(([k, label]) => (
                <button
                  key={k}
                  type="button"
                  onClick={() => setFilter(k)}
                  style={{
                    all: 'unset',
                    cursor: 'default',
                    padding: '3px 10px',
                    fontFamily: '"DotGothic16", monospace',
                    fontSize: 11,
                    letterSpacing: 1,
                    textTransform: 'uppercase',
                    color: filter === k ? '#080814' : theme.dim,
                    background: filter === k ? theme.gold : 'transparent',
                    border: `2px solid ${filter === k ? theme.gold : theme.dim + '88'}`,
                  }}
                >
                  {label}
                </button>
              ))}
              <div style={{ flex: 1 }} />
              <span style={{ fontFamily: '"DotGothic16", monospace', fontSize: 11, color: theme.dim, letterSpacing: 1, alignSelf: 'center' }}>
                {filteredQuests.length} / {data.quests.length}
              </span>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column' }}>
              {filteredQuests.length === 0 && (
                <div style={{ padding: 24, textAlign: 'center', fontFamily: 'Galmuri11, "DotGothic16", monospace', color: theme.dim, fontSize: 13 }}>
                  {i18n.emptyTavern}
                </div>
              )}
              {filteredQuests.map((q, i) => (
                <QuestRow
                  key={q.id}
                  theme={theme}
                  quest={q}
                  lang={lang}
                  active={i === questCursor && focus === 'quests'}
                  onClick={() => { setQuestId(q.id); setFocus('quests'); setQuestCursor(i); }}
                />
              ))}
            </div>
          </Window>
        </div>

        {/* Tavern log */}
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 320px', gap: 14 }}>
          <Window theme={theme} title={i18n.tabs.tavern} accent="LIVE FEED">
            <div style={{ maxHeight: 320, overflowY: 'auto', paddingRight: 4 }}>
              {data.events.map((ev, i) => (
                <EventLogLine key={i} theme={theme} ev={ev} />
              ))}
            </div>
            <div style={{ marginTop: 10 }}>
              <DialogBox
                theme={theme}
                lines={['🍺 주점 주인', '오늘은 의뢰가 많네요. 가장 위급한 건 #141이에요 — 승인을 기다리고 있어요.']}
              />
            </div>
          </Window>

          {/* Chronicle widget */}
          <Window theme={theme} title={i18n.tabs.chronicle} accent="3 PATTERNS">
            <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
              {data.patterns.map((p) => (
                <PatternCard key={p.id} theme={theme} lang={lang} p={p} />
              ))}
            </div>
          </Window>
        </div>
      </div>

      {/* Keyboard hint footer */}
      <div
        style={{
          background: theme.win,
          border: `2px solid ${theme.border}`,
          padding: 3,
        }}
      >
        <div
          style={{
            border: `2px solid ${theme.border}`,
            padding: '6px 12px',
            display: 'flex',
            gap: 18,
            fontFamily: '"DotGothic16", monospace',
            fontSize: 11,
            color: theme.dim,
            letterSpacing: 1,
            flexWrap: 'wrap',
          }}
        >
          <span><b style={{ color: theme.gold }}>j/k</b> 이동</span>
          <span><b style={{ color: theme.gold }}>↵</b> 선택</span>
          <span><b style={{ color: theme.gold }}>g q</b> 의뢰 패널</span>
          <span><b style={{ color: theme.gold }}>g a</b> 모험가</span>
          <span><b style={{ color: theme.gold }}>⌘K</b> 커맨드 팔레트</span>
          <span><b style={{ color: theme.gold }}>esc</b> 닫기</span>
          <span style={{ flex: 1 }} />
          <span style={{ color: theme.dim }}>tavern.db — ~/.luida/tavern.db · port 4321</span>
        </div>
      </div>

      {selectedQuest && (
        <QuestDetailModal
          theme={theme}
          lang={lang}
          quest={selectedQuest}
          onClose={() => setQuestId(null)}
          onApprove={() => {
            onShowToast(`✓ 의뢰 #${selectedQuest.id}를 승인했어요. ${selectedQuest.to}에게 진행을 알립니다.`);
            setQuestId(null);
          }}
          onReject={() => {
            onShowToast(`🌀 의뢰 #${selectedQuest.id}을 거절했습니다.`);
            setQuestId(null);
          }}
        />
      )}

      {cmdOpen && <CommandPalette theme={theme} onClose={() => setCmdOpen(false)} />}
    </div>
  );
}

function QuestDetailModal({ theme, lang, quest, onClose, onApprove, onReject }) {
  const i18n = window.LUIDA_I18N[lang];
  const needsApproval = quest.status === 'needs_approval';
  const logLines = [
    `[14:18:02] ${quest.to}: 시작합니다 — git checkout -b ${quest.branch}`,
    `[14:18:11] tool_used: read_file(prisma/schema.prisma)`,
    `[14:18:24] tool_used: grep("chronicle_events")`,
    `[14:18:51] tool_used: write_file(prisma/schema.prisma)`,
    `[14:19:02] tool_used: bash("bunx prisma migrate dev --create-only")`,
    `[14:19:33] ${quest.to}: 마이그레이션 파일 생성 — migrations/20260526_chronicle_events`,
    `[14:19:48] tool_used: write_file(packages/core/src/schema.ts)`,
    `[14:20:11] review: typecheck ok · ${quest.progress_label}`,
  ];
  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 50,
        background: 'rgba(0,8,20,0.78)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: 24,
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{ width: 720, maxWidth: '100%', maxHeight: '90vh', display: 'flex', flexDirection: 'column' }}
      >
        <Window
          theme={theme}
          title={`의뢰서 #${quest.id}`}
          accent={`@${quest.to} · ${quest.branch}`}
          style={{ flex: 1, display: 'flex', flexDirection: 'column' }}
        >
          <div style={{ display: 'flex', flexDirection: 'column', gap: 12, overflow: 'auto', maxHeight: '70vh' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <Badge theme={theme} kind={quest.status} label={i18n.statusBadges[quest.status]} size="lg" />
              <span style={{ fontFamily: '"DotGothic16", monospace', fontSize: 11, color: theme.dim, letterSpacing: 1 }}>
                · {quest.created_label}
              </span>
            </div>

            <DialogBox
              theme={theme}
              lines={['📜 의뢰서 내용', quest.brief]}
            />

            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
              {[
                ['BRANCH', quest.branch],
                ['WORKTREE', `~/wt/${quest.to}/${quest.branch.split('/').pop()}`],
                ['DISPATCHED BY', 'luida'],
                ['PR', quest.pr_url ? quest.pr_url.replace('https://', '') : '—'],
              ].map(([k, v]) => (
                <div key={k} style={{ borderLeft: `2px solid ${theme.gold}`, paddingLeft: 8 }}>
                  <div style={{ fontFamily: '"DotGothic16", monospace', fontSize: 10, color: theme.gold, letterSpacing: 1 }}>
                    {k}
                  </div>
                  <div style={{ fontFamily: '"DotGothic16", monospace', fontSize: 12, color: theme.text, marginTop: 2, wordBreak: 'break-all' }}>
                    {v}
                  </div>
                </div>
              ))}
            </div>

            <div>
              <div style={{ fontFamily: '"DotGothic16", monospace', fontSize: 11, color: theme.gold, letterSpacing: 1, marginBottom: 6 }}>
                PROGRESS · {quest.progress_pct}%
              </div>
              <ProgressStrip theme={theme} pct={quest.progress_pct} status={quest.status} />
              <div style={{ fontFamily: 'Galmuri11, "DotGothic16", monospace', fontSize: 12, color: theme.dim, marginTop: 4 }}>
                {quest.progress_label}
              </div>
            </div>

            <div>
              <div style={{ fontFamily: '"DotGothic16", monospace', fontSize: 11, color: theme.gold, letterSpacing: 1, marginBottom: 6 }}>
                LOG (stream-json)
              </div>
              <div
                style={{
                  background: theme.bg,
                  border: `1px solid ${theme.dim}55`,
                  padding: '10px 12px',
                  fontFamily: '"DotGothic16", monospace',
                  fontSize: 11,
                  color: theme.dim,
                  lineHeight: 1.65,
                  maxHeight: 180,
                  overflow: 'auto',
                }}
              >
                {logLines.map((l, i) => (
                  <div key={i}>
                    <span style={{ color: theme.text }}>{l}</span>
                  </div>
                ))}
              </div>
            </div>

            {needsApproval && (
              <div
                style={{
                  background: theme.win,
                  border: `2px solid ${theme.pink}`,
                  padding: '10px 12px',
                  display: 'flex',
                  alignItems: 'center',
                  gap: 12,
                  flexWrap: 'wrap',
                }}
              >
                <div style={{ fontFamily: 'Galmuri11, "DotGothic16", monospace', color: theme.text, fontSize: 13, flex: 1 }}>
                  <span style={{ color: theme.pink }}>⚠ </span>
                  이 의뢰는 사용자 승인을 기다리고 있어요. 진행할까요?
                </div>
                <PixelButton theme={theme} variant="success" onClick={onApprove}>
                  {i18n.cta.approve}
                </PixelButton>
                <PixelButton theme={theme} variant="danger" onClick={onReject}>
                  {i18n.cta.reject}
                </PixelButton>
              </div>
            )}

            <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
              <PixelButton theme={theme} variant="ghost" onClick={onClose}>
                닫기 (esc)
              </PixelButton>
            </div>
          </div>
        </Window>
      </div>
    </div>
  );
}

function CommandPalette({ theme, onClose }) {
  const [q, setQ] = useState_D('');
  const items = [
    { label: 'agora에 새 의뢰 보내기', hint: 'dispatch agora' },
    { label: 'admin에 schema 마이그레이션 의뢰', hint: 'dispatch admin' },
    { label: 'kontrol에 staging 점검 의뢰', hint: 'dispatch kontrol' },
    { label: '#141 의뢰 승인', hint: 'approve 141' },
    { label: '대시보드로 이동', hint: 'g d' },
    { label: '연감 열기', hint: 'g c' },
    { label: '관계 그래프', hint: 'g r' },
    { label: '설정', hint: ', settings' },
  ];
  const filtered = items.filter((it) => it.label.toLowerCase().includes(q.toLowerCase()) || it.hint.includes(q.toLowerCase()));
  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 60,
        background: 'rgba(0,8,20,0.78)',
        display: 'flex',
        alignItems: 'flex-start',
        justifyContent: 'center',
        paddingTop: 96,
      }}
    >
      <div onClick={(e) => e.stopPropagation()} style={{ width: 560 }}>
        <Window theme={theme} title="커맨드 팔레트" accent="⌘K">
          <input
            autoFocus
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder="자연어로 명령하세요 — '의뢰 보내', 'agora 호출'…"
            style={{
              all: 'unset',
              display: 'block',
              width: '100%',
              padding: '8px 10px',
              fontFamily: 'Galmuri11, "DotGothic16", monospace',
              fontSize: 14,
              color: theme.text,
              background: theme.bg,
              border: `2px solid ${theme.gold}`,
              marginBottom: 10,
              boxSizing: 'border-box',
            }}
          />
          <MenuList
            theme={theme}
            value={filtered[0]?.label}
            items={filtered.map((it) => ({ value: it.label, label: it.label, hint: it.hint }))}
            onChange={() => {}}
            onSelect={onClose}
          />
        </Window>
      </div>
    </div>
  );
}

Object.assign(window, { DashboardTab, QuestDetailModal, CommandPalette });
