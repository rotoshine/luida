// @ts-nocheck
// Transitional Vite migration — types tightened later.
import React, { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { Window, DialogBox, MenuList, Badge, StatusBar, ProgressStrip, PixelButton } from './primitives';
import { LUIDA_TOKENS, LUIDA_I18N, LUIDA_SEED_ADVENTURERS, LUIDA_SEED_QUESTS, LUIDA_SEED_EVENTS, LUIDA_SEED_PATTERNS, LUIDA_DATA } from './data';
// Composite cards: AdventurerCard, QuestRow, EventLogLine, PatternCard, Toast.



function AdventurerCard({ theme, adv, lang, onClick, selected }) {
  const i18n = LUIDA_I18N[lang];
  const dim = adv.status === 'offline';
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        all: 'unset',
        cursor: 'default',
        display: 'block',
        background: selected ? theme.winAlt : theme.win,
        border: `2px solid ${selected ? theme.gold : theme.border}`,
        padding: 10,
        opacity: dim ? 0.55 : 1,
        boxSizing: 'border-box',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 10 }}>
        <div
          style={{
            width: 40,
            height: 40,
            background: theme.bg,
            border: `2px solid ${theme.border}`,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            fontSize: 22,
            flexShrink: 0,
            fontFamily: '"DotGothic16", monospace',
          }}
        >
          {adv.icon}
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: 6 }}>
            <span
              style={{
                fontFamily: '"DotGothic16", monospace',
                fontSize: 14,
                color: theme.gold,
                letterSpacing: 1,
                textTransform: 'uppercase',
              }}
            >
              {adv.name}
            </span>
            <span style={{ fontFamily: '"DotGothic16", monospace', fontSize: 10, color: theme.dim, letterSpacing: 1 }}>
              Lv.{String(adv.level).padStart(2, '0')}
            </span>
          </div>
          <div
            style={{
              fontFamily: 'Galmuri11, "DotGothic16", monospace',
              fontSize: 12,
              color: theme.dim,
              marginTop: 1,
              marginBottom: 8,
            }}
          >
            {lang === 'ko' ? adv.classKr : adv.class}
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
            <StatusBar theme={theme} label="HP" current={adv.hp.current} max={adv.hp.max} />
            <StatusBar theme={theme} label="MP" current={adv.mp.current} max={adv.mp.max} color="mp" />
          </div>
          <div
            style={{
              marginTop: 8,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              gap: 6,
              fontFamily: '"DotGothic16", monospace',
              fontSize: 11,
            }}
          >
            <Badge theme={theme} kind={adv.status} label={i18n.statusBadgesAdv[adv.status]} />
            <span style={{ color: theme.dim }}>
              {adv.current_quest ? (
                <span>
                  <span style={{ color: theme.gold }}>QUEST</span> #{adv.current_quest}
                </span>
              ) : (
                <span>—</span>
              )}
            </span>
          </div>
        </div>
      </div>
    </button>
  );
}

function QuestRow({ theme, quest, lang, onClick, active }) {
  const i18n = LUIDA_I18N[lang];
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        all: 'unset',
        cursor: 'default',
        display: 'grid',
        gridTemplateColumns: '64px 1fr 110px 140px',
        gap: 10,
        alignItems: 'center',
        padding: '8px 10px',
        background: active ? theme.winAlt : 'transparent',
        borderLeft: `4px solid ${active ? theme.gold : 'transparent'}`,
        borderBottom: `1px solid ${theme.dim}22`,
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          fontFamily: '"DotGothic16", monospace',
          fontSize: 13,
          color: theme.gold,
        }}
      >
        <span style={{ color: active ? theme.gold : 'transparent' }}>▶</span>
        <span>#{quest.id}</span>
      </div>
      <div style={{ minWidth: 0 }}>
        <div
          style={{
            fontFamily: 'Galmuri11, "DotGothic16", monospace',
            fontSize: 13,
            color: theme.text,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {quest.brief}
        </div>
        <div
          style={{
            fontFamily: '"DotGothic16", monospace',
            fontSize: 10,
            color: theme.dim,
            marginTop: 2,
            letterSpacing: 0.5,
            display: 'flex',
            gap: 8,
          }}
        >
          <span style={{ color: theme.pink }}>@{quest.to}</span>
          <span>{quest.branch}</span>
          <span style={{ color: theme.dim }}>· {quest.created_label}</span>
        </div>
      </div>
      <div>
        <Badge theme={theme} kind={quest.status} label={i18n.statusBadges[quest.status]} />
      </div>
      <div>
        <ProgressStrip theme={theme} pct={quest.progress_pct} status={quest.status} />
        <div style={{ fontFamily: '"DotGothic16", monospace', fontSize: 10, color: theme.dim, marginTop: 3 }}>
          {quest.progress_label}
        </div>
      </div>
    </button>
  );
}

function EventLogLine({ theme, ev }) {
  const tone = {
    dispatch: theme.blue,
    progress: theme.dim,
    alert: theme.red,
    ack: theme.green,
    proposal: theme.pink,
    info: theme.dim,
    aborted: theme.yellow,
  }[ev.kind] || theme.dim;
  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: '54px 22px 1fr',
        gap: 6,
        alignItems: 'baseline',
        padding: '4px 0',
        fontFamily: 'Galmuri11, "DotGothic16", monospace',
        fontSize: 12,
        lineHeight: 1.45,
        color: theme.text,
        borderBottom: `1px dashed ${theme.dim}22`,
      }}
    >
      <span style={{ fontFamily: '"DotGothic16", monospace', color: theme.dim, fontSize: 11 }}>
        {ev.t}
      </span>
      <span style={{ color: tone, fontSize: 13 }}>{ev.icon}</span>
      <span>{ev.text}</span>
    </div>
  );
}

function PatternCard({ theme, lang, p }) {
  const i18n = LUIDA_I18N[lang];
  const active = p.kind === 'active';
  const conf = p.confidence;
  return (
    <div
      style={{
        background: active ? theme.win : theme.win,
        border: `2px solid ${active ? theme.green : theme.gold}`,
        padding: 10,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: 8 }}>
        <div
          style={{
            fontFamily: 'Galmuri11, "DotGothic16", monospace',
            color: active ? theme.green : theme.gold,
            fontSize: 13,
            letterSpacing: 0.5,
          }}
        >
          {active ? '✓ 활성 룰' : '💡 패턴 후보'}
        </div>
        {conf != null && (
          <div style={{ fontFamily: '"DotGothic16", monospace', color: theme.dim, fontSize: 10 }}>
            {'★'.repeat(conf)}
            <span style={{ color: theme.dim + '55' }}>{'★'.repeat(10 - conf)}</span>
          </div>
        )}
      </div>
      <div
        style={{
          fontFamily: '"DotGothic16", monospace',
          fontSize: 12,
          color: theme.text,
          letterSpacing: 0.5,
          marginTop: 4,
        }}
      >
        {p.title}
      </div>
      <div
        style={{
          fontFamily: 'Galmuri11, "DotGothic16", monospace',
          fontSize: 12,
          color: theme.dim,
          marginTop: 4,
          lineHeight: 1.5,
        }}
      >
        {p.desc}
      </div>
      {!active && (
        <div style={{ display: 'flex', gap: 6, marginTop: 8 }}>
          <PixelButton theme={theme} size="sm" variant="primary">
            {i18n.cta.promote}
          </PixelButton>
          <PixelButton theme={theme} size="sm" variant="ghost">
            {i18n.cta.dismiss}
          </PixelButton>
        </div>
      )}
    </div>
  );
}

function Toast({ theme, message, onDismiss }) {
  return (
    <div
      style={{
        position: 'fixed',
        top: 24,
        right: 24,
        zIndex: 100,
        width: 360,
        animation: 'luida-toast-in 280ms steps(6) both',
      }}
    >
      <Window theme={theme} title="알림" accent="NEW">
        <div style={{ fontFamily: 'Galmuri11, "DotGothic16", monospace', fontSize: 13, color: theme.text }}>
          {message}
        </div>
        <div style={{ marginTop: 10, display: 'flex', justifyContent: 'flex-end' }}>
          <PixelButton theme={theme} size="sm" onClick={onDismiss}>
            확인
          </PixelButton>
        </div>
      </Window>
    </div>
  );
}

export { AdventurerCard, QuestRow, EventLogLine, PatternCard, Toast };
