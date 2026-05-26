// @ts-nocheck
// Transitional Vite migration — types tightened later.
import React, { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { Window, DialogBox, MenuList, Badge, StatusBar, PixelButton } from './primitives';
import { AdventurerCard, QuestRow, EventLogLine, PatternCard } from './cards';
import { LUIDA_TOKENS, LUIDA_I18N, LUIDA_SEED_ADVENTURERS, LUIDA_SEED_QUESTS, LUIDA_SEED_EVENTS, LUIDA_SEED_PATTERNS, LUIDA_DATA } from './data';
// Three "design system" tabs: Moodboard, Tokens, Components. Plus shared Section helper.



function SectionTitle({ theme, idx, label, sub }) {
  return (
    <div style={{ marginBottom: 18 }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'baseline',
          gap: 10,
          fontFamily: '"DotGothic16", monospace',
          color: theme.gold,
          letterSpacing: 2,
          fontSize: 12,
        }}
      >
        <span>{idx}</span>
        <span style={{ flex: 1, borderBottom: `1px dashed ${theme.dim}55`, transform: 'translateY(-4px)' }} />
      </div>
      <div
        style={{
          fontFamily: 'Galmuri11, "DotGothic16", monospace',
          fontSize: 22,
          color: theme.text,
          marginTop: 4,
        }}
      >
        {label}
      </div>
      {sub && (
        <div
          style={{
            fontFamily: 'Galmuri11, "DotGothic16", monospace',
            fontSize: 13,
            color: theme.dim,
            marginTop: 4,
            maxWidth: 720,
            lineHeight: 1.6,
          }}
        >
          {sub}
        </div>
      )}
    </div>
  );
}

// ── 1. MOODBOARD ─────────────────────────────────────────────────────────────
function MoodboardTab({ theme, lang }) {
  const principles = [
    { t: '순수한 검정 + 흰 더블 라인', d: '직각 테두리만 사용. 라운드·그라데이션·소프트 섀도는 금기.' },
    { t: '픽셀 폰트 두 종 혼용', d: 'Galmuri11(한글) + DotGothic16(영문/숫자). 본문도 픽셀.' },
    { t: '커서는 ▶', d: '메뉴 선택 표식. 행 좌측 14px 고정 슬롯, 비활성행에도 자리 비워두기.' },
    { t: 'HP·MP는 셀 단위 막대', d: '20셀 픽셀 블록. % 보간 없이 셀이 차거나 비거나.' },
    { t: 'NPC 화법', d: '"의뢰가 게시판에 붙었어요" / "모험가가 쓰러졌습니다". 단, 가짜 게이미피케이션 금지.' },
    { t: '운영성이 1순위', d: '한눈에 상태 파악이 안 되면 RPG 감성을 양보한다.' },
  ];
  const refs = [
    { tag: 'GAME MENU', label: '레트로 RPG 주점 메뉴', note: '순수한 검정 / 더블 라인 / ▶ 커서' },
    { tag: 'STATUS', label: '파티 스테이터스 화면', note: 'HP/MP 바 · 직업 · 레벨' },
    { tag: 'DIALOG', label: '메시지 윈도우', note: '말풍선 ▼ 인디케이터 · 글자 한 자씩 떨어지는 효과' },
    { tag: 'NES.CSS', label: 'NES.css 갤러리', note: '대안 톤 — 밝은 회색 + 검정 테두리' },
    { tag: '98.CSS', label: 'Windows 98 UI', note: '대안 톤 — 회색 베젤 + 인셋' },
    { tag: 'PSone.CSS', label: 'PS1 메뉴', note: '대안 톤 — 슬랜트 / 텍스처' },
  ];
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 28 }}>
      <SectionTitle
        theme={theme}
        idx="01 / MOODBOARD"
        label="루이다의 주점을 짓는 첫걸음"
        sub="레트로 RPG의 메뉴 UI 컨벤션을 빌려서 운영 대시보드를 만듭니다. 기존 게임의 보호되는 비주얼은 베끼지 않고, 픽셀 폰트와 직각 더블라인이라는 일반 어휘만 차용해 원작 게임과는 다른 컬러·구성·정보 밀도로 재해석합니다."
      />

      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 16 }}>
        {refs.map((r) => (
          <div key={r.label} style={{ background: theme.win, border: `2px solid ${theme.border}`, padding: 3 }}>
            <div style={{ border: `2px solid ${theme.border}`, padding: 0 }}>
              <div
                style={{
                  aspectRatio: '4 / 3',
                  background: `repeating-linear-gradient(135deg, ${theme.winAlt} 0 8px, ${theme.win} 8px 16px)`,
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  position: 'relative',
                  borderBottom: `2px solid ${theme.border}`,
                }}
              >
                <div
                  style={{
                    position: 'absolute',
                    top: 8,
                    left: 8,
                    fontFamily: '"DotGothic16", monospace',
                    fontSize: 10,
                    color: theme.gold,
                    background: theme.bg,
                    padding: '2px 6px',
                    letterSpacing: 1,
                  }}
                >
                  {r.tag}
                </div>
                <div
                  style={{
                    fontFamily: '"DotGothic16", monospace',
                    fontSize: 11,
                    color: theme.dim,
                    background: theme.bg,
                    padding: '4px 8px',
                  }}
                >
                  reference image
                </div>
              </div>
              <div style={{ padding: 10 }}>
                <div style={{ fontFamily: 'Galmuri11, "DotGothic16", monospace', fontSize: 13, color: theme.text }}>
                  {r.label}
                </div>
                <div style={{ fontFamily: 'Galmuri11, "DotGothic16", monospace', fontSize: 11, color: theme.dim, marginTop: 3 }}>
                  {r.note}
                </div>
              </div>
            </div>
          </div>
        ))}
      </div>

      <SectionTitle theme={theme} idx="02 / PRINCIPLES" label="여섯 가지 원칙" />
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 12 }}>
        {principles.map((p, i) => (
          <div key={i} style={{ background: theme.win, border: `2px solid ${theme.border}`, padding: 3 }}>
            <div style={{ border: `2px solid ${theme.border}`, padding: '10px 12px' }}>
              <div style={{ fontFamily: '"DotGothic16", monospace', color: theme.gold, fontSize: 11, letterSpacing: 1 }}>
                {String(i + 1).padStart(2, '0')}
              </div>
              <div
                style={{
                  fontFamily: 'Galmuri11, "DotGothic16", monospace',
                  color: theme.text,
                  fontSize: 14,
                  marginTop: 2,
                }}
              >
                {p.t}
              </div>
              <div
                style={{
                  fontFamily: 'Galmuri11, "DotGothic16", monospace',
                  color: theme.dim,
                  fontSize: 12,
                  marginTop: 6,
                  lineHeight: 1.55,
                }}
              >
                {p.d}
              </div>
            </div>
          </div>
        ))}
      </div>

      <SectionTitle theme={theme} idx="03 / VOICE" label="주점 NPC 화법 샘플" />
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, 1fr)', gap: 12 }}>
        {[
          ['🍺 ack', '모험가 admin이 의뢰 #142를 완수하고 돌아왔어요.'],
          ['📜 dispatch', '새 의뢰가 게시판에 붙었습니다 — admin 행 #143.'],
          ['⚠ alert', 'kontrol이 함정에 빠진 듯합니다 (typecheck 실패).'],
          ['💀 down', '모험가가 쓰러졌습니다. HP를 회복할 시간입니다.'],
          ['🌀 abort', '의뢰가 무효화되었습니다. 사용자가 길을 바꾸셨군요.'],
          ['🌙 empty', '오늘은 평화로운 하루입니다. 주점이 조용하네요.'],
        ].map(([k, t]) => (
          <DialogBox key={k} theme={theme} lines={[k.toUpperCase(), t]} />
        ))}
      </div>
    </div>
  );
}

// ── 2. TOKENS ────────────────────────────────────────────────────────────────
function TokensTab({ theme, lang }) {
  const swatches = [
    { name: 'bg', token: '--bg', hex: theme.bg, note: '가장 깊은 바깥 배경' },
    { name: 'win', token: '--win', hex: theme.win, note: '윈도우 본체' },
    { name: 'winAlt', token: '--win-alt', hex: theme.winAlt, note: '윈도우 강조 / 선택 상태' },
    { name: 'border', token: '--border', hex: theme.border, note: '더블라인 테두리' },
    { name: 'text', token: '--text', hex: theme.text, note: '본문' },
    { name: 'dim', token: '--dim', hex: theme.dim, note: '보조 텍스트' },
    { name: 'gold', token: '--gold', hex: theme.gold, note: '강조 · 헤더 · 커서' },
    { name: 'green', token: '--hp-green', hex: theme.green, note: 'HP 양호 · 완료' },
    { name: 'yellow', token: '--hp-yellow', hex: theme.yellow, note: 'HP 주의 · 검토' },
    { name: 'red', token: '--hp-red', hex: theme.red, note: 'HP 위험 · 실패' },
    { name: 'blue', token: '--mp', hex: theme.blue, note: 'MP · 진행중' },
    { name: 'pink', token: '--accent', hex: theme.pink, note: '이벤트 · 승인 필요' },
  ];
  const typeScale = [
    { name: 'display', size: 28, family: 'Galmuri11', sample: '루이다의 주점 · LUIDA' },
    { name: 'h1', size: 20, family: 'Galmuri11', sample: '의뢰 게시판 · QUEST BOARD' },
    { name: 'h2', size: 16, family: 'Galmuri11', sample: '모험가 admin · Lv.31' },
    { name: 'body', size: 14, family: 'Galmuri11', sample: 'Prisma 스키마에 chronicle_events 추가' },
    { name: 'caption', size: 12, family: 'Galmuri11', sample: '38분 전 · feat/chronicle-events' },
    { name: 'mono', size: 11, family: 'DotGothic16', sample: 'QUEST #143 · NEEDS_APPROVAL' },
  ];
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 28 }}>
      <SectionTitle
        theme={theme}
        idx="01 / COLOR"
        label="컬러 팔레트"
        sub="모든 테마는 동일한 토큰 구조를 갖습니다. 우측 Tweaks 패널에서 테마를 전환해 같은 화면에 다른 컬러가 그대로 입혀지는지 확인하세요. 토큰 이름은 packages/web/src/design/tokens.ts에서 그대로 export됩니다."
      />
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(6, 1fr)', gap: 10 }}>
        {swatches.map((s) => (
          <div key={s.name} style={{ background: theme.win, border: `2px solid ${theme.border}`, padding: 3 }}>
            <div style={{ border: `2px solid ${theme.border}` }}>
              <div style={{ height: 72, background: s.hex, borderBottom: `2px solid ${theme.border}` }} />
              <div style={{ padding: '8px 10px' }}>
                <div style={{ fontFamily: '"DotGothic16", monospace', fontSize: 12, color: theme.gold, letterSpacing: 1 }}>
                  {s.name}
                </div>
                <div style={{ fontFamily: '"DotGothic16", monospace', fontSize: 10, color: theme.dim, marginTop: 2 }}>
                  {s.hex.toUpperCase()}
                </div>
                <div style={{ fontFamily: 'Galmuri11, "DotGothic16", monospace', fontSize: 11, color: theme.text, marginTop: 6 }}>
                  {s.note}
                </div>
              </div>
            </div>
          </div>
        ))}
      </div>

      <SectionTitle theme={theme} idx="02 / TYPOGRAPHY" label="타이포그래피 스케일" />
      <div style={{ background: theme.win, border: `2px solid ${theme.border}`, padding: 3 }}>
        <div style={{ border: `2px solid ${theme.border}`, padding: '14px 18px' }}>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
            {typeScale.map((t) => (
              <div key={t.name} style={{ display: 'grid', gridTemplateColumns: '120px 80px 100px 1fr', gap: 16, alignItems: 'baseline' }}>
                <div style={{ fontFamily: '"DotGothic16", monospace', color: theme.gold, fontSize: 11, letterSpacing: 1 }}>
                  {t.name}
                </div>
                <div style={{ fontFamily: '"DotGothic16", monospace', color: theme.dim, fontSize: 11 }}>
                  {t.size}px
                </div>
                <div style={{ fontFamily: '"DotGothic16", monospace', color: theme.dim, fontSize: 11 }}>
                  {t.family}
                </div>
                <div
                  style={{
                    fontFamily: t.family === 'DotGothic16' ? '"DotGothic16", monospace' : 'Galmuri11, "DotGothic16", monospace',
                    fontSize: t.size,
                    color: theme.text,
                  }}
                >
                  {t.sample}
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>

      <SectionTitle theme={theme} idx="03 / SPACING" label="간격 · 라인" />
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(6, 1fr)', gap: 10 }}>
        {[2, 4, 8, 12, 16, 24].map((v) => (
          <div key={v} style={{ background: theme.win, border: `2px solid ${theme.border}`, padding: 3 }}>
            <div style={{ border: `2px solid ${theme.border}`, padding: 12 }}>
              <div style={{ fontFamily: '"DotGothic16", monospace', color: theme.gold, fontSize: 11 }}>space-{v}</div>
              <div style={{ height: v, background: theme.gold, marginTop: 8 }} />
              <div style={{ fontFamily: '"DotGothic16", monospace', color: theme.dim, fontSize: 10, marginTop: 6 }}>
                {v}px
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

// ── 3. COMPONENT CATALOG ─────────────────────────────────────────────────────
function ComponentsTab({ theme, lang }) {
  const [menuVal, setMenuVal] = useState_S('dispatch');
  const data = LUIDA_DATA;
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 28 }}>
      <SectionTitle
        theme={theme}
        idx="01 / CONTAINERS"
        label="윈도우 · 다이얼로그"
        sub="모든 컨테이너는 직각 더블라인. 본문에는 padding 16px, 헤더 영역과 본문 사이는 1px dashed 라인으로 분리."
      />
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
        <Window theme={theme} title="모험가" accent="6 / 6">
          <div style={{ fontFamily: 'Galmuri11, "DotGothic16", monospace', fontSize: 13, color: theme.text, lineHeight: 1.6 }}>
            등록된 모험가가 6명 모여 있습니다. 카드 1장에 이름·직업·HP/MP·현재 의뢰 ID가 모두 들어갑니다.
          </div>
        </Window>
        <DialogBox
          theme={theme}
          lines={[
            '🍺 주점 주인',
            '오늘도 의뢰가 많이 들어왔어요, 용사여.',
            '게시판부터 살펴보시겠어요?',
          ]}
        />
      </div>

      <SectionTitle theme={theme} idx="02 / LISTS" label="메뉴 리스트 · 의뢰 행" />
      <div style={{ display: 'grid', gridTemplateColumns: '320px 1fr', gap: 16 }}>
        <Window theme={theme} title="커맨드">
          <MenuList
            theme={theme}
            value={menuVal}
            onChange={setMenuVal}
            items={[
              { value: 'dispatch', label: '의뢰 보내기', hint: 'd' },
              { value: 'register', label: '모험가 모집', hint: 'r' },
              { value: 'log', label: '주점 로그', hint: 'l' },
              { value: 'chronicle', label: '연감 열기', hint: 'c' },
              { value: 'settings', label: '설정', hint: ',' },
              { value: 'quit', label: '문을 닫는다', hint: 'q', danger: true },
            ]}
          />
        </Window>
        <Window theme={theme} title="의뢰 게시판 (행 샘플)">
          <div style={{ display: 'flex', flexDirection: 'column' }}>
            {data.quests.slice(0, 4).map((q) => (
              <QuestRow key={q.id} theme={theme} quest={q} lang={lang} />
            ))}
          </div>
        </Window>
      </div>

      <SectionTitle theme={theme} idx="03 / STATUS" label="HP·MP 바 · 뱃지" />
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 16 }}>
        <Window theme={theme} title="HP / MP">
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <StatusBar theme={theme} label="HP" current={84} max={100} />
            <StatusBar theme={theme} label="HP" current={41} max={100} />
            <StatusBar theme={theme} label="HP" current={9} max={100} />
            <StatusBar theme={theme} label="MP" current={32} max={50} color="mp" />
          </div>
        </Window>
        <Window theme={theme} title="상태 뱃지">
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
            {['running', 'reviewing', 'needs_approval', 'pr_ready', 'completed', 'failed', 'aborted', 'pending'].map((k) => (
              <Badge key={k} theme={theme} kind={k} label={LUIDA_I18N[lang].statusBadges[k]} />
            ))}
          </div>
          <div style={{ height: 10 }} />
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
            {['busy', 'idle', 'offline'].map((k) => (
              <Badge key={k} theme={theme} kind={k} label={LUIDA_I18N[lang].statusBadgesAdv[k]} />
            ))}
          </div>
        </Window>
        <Window theme={theme} title="버튼">
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8, alignItems: 'flex-start' }}>
            <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
              <PixelButton theme={theme} variant="primary">승인</PixelButton>
              <PixelButton theme={theme} variant="danger">거절</PixelButton>
              <PixelButton theme={theme}>닫기</PixelButton>
              <PixelButton theme={theme} variant="ghost">취소</PixelButton>
            </div>
            <div style={{ display: 'flex', gap: 6 }}>
              <PixelButton theme={theme} size="sm">SM</PixelButton>
              <PixelButton theme={theme} size="md">MD</PixelButton>
              <PixelButton theme={theme} size="lg">LG</PixelButton>
            </div>
          </div>
        </Window>
      </div>

      <SectionTitle theme={theme} idx="04 / CARDS" label="모험가 카드 · 이벤트 라인 · 패턴 카드" />
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 16 }}>
        <AdventurerCard theme={theme} adv={data.adventurers[0]} lang={lang} />
        <AdventurerCard theme={theme} adv={data.adventurers[3]} lang={lang} />
        <AdventurerCard theme={theme} adv={data.adventurers[4]} lang={lang} />
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: '2fr 1fr', gap: 16 }}>
        <Window theme={theme} title="주점 게시판 (이벤트 라인 4개)">
          {data.events.slice(0, 4).map((ev, i) => (
            <EventLogLine key={i} theme={theme} ev={ev} />
          ))}
        </Window>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          {data.patterns.slice(0, 2).map((p) => (
            <PatternCard key={p.id} theme={theme} lang={lang} p={p} />
          ))}
        </div>
      </div>
    </div>
  );
}

export { MoodboardTab, TokensTab, ComponentsTab, SectionTitle };
