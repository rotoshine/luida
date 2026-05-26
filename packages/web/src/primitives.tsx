// @ts-nocheck
// Transitional Vite migration — types tightened later.
import React, { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { LUIDA_TOKENS, LUIDA_I18N, LUIDA_SEED_ADVENTURERS, LUIDA_SEED_QUESTS, LUIDA_SEED_EVENTS, LUIDA_SEED_PATTERNS, LUIDA_DATA } from './data';
// Core retro-RPG primitives: Window, DialogBox, MenuList, Badge, StatusBar, Cursor.
// All styled inline so they pick up the live theme prop without needing CSS vars per call.



// Generic double-line bordered window — the workhorse container.
function Window({ theme, title, children, style, padded = true, hp, accent }) {
  const outerStyle = {
    background: theme.win,
    border: `2px solid ${theme.border}`,
    boxShadow: '0 0 0 0 transparent',
    position: 'relative',
    ...style,
  };
  const innerStyle = {
    border: `2px solid ${theme.border}`,
    padding: padded ? '14px 16px 16px' : 0,
    minHeight: 0,
    display: 'flex',
    flexDirection: 'column',
    flex: 1,
    overflow: 'hidden',
  };
  return (
    <div className="luida-window" style={outerStyle}>
      <div style={{ padding: 3, display: 'flex', flexDirection: 'column', height: '100%', boxSizing: 'border-box' }}>
        <div style={innerStyle}>
          {title && (
            <div
              style={{
                display: 'flex',
                alignItems: 'baseline',
                justifyContent: 'space-between',
                gap: 12,
                marginBottom: 12,
                paddingBottom: 8,
                borderBottom: `2px dashed ${theme.dim}33`,
              }}
            >
              <div
                style={{
                  fontFamily: 'Galmuri11, "DotGothic16", monospace',
                  fontSize: 14,
                  color: theme.gold,
                  letterSpacing: 1,
                  textTransform: 'uppercase',
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                }}
              >
                <span style={{ color: theme.border }}>▶</span>
                {title}
              </div>
              {accent && (
                <div
                  style={{
                    fontFamily: '"DotGothic16", monospace',
                    fontSize: 11,
                    color: theme.dim,
                    letterSpacing: 1,
                  }}
                >
                  {accent}
                </div>
              )}
            </div>
          )}
          <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
            {children}
          </div>
        </div>
      </div>
    </div>
  );
}

// DQ-style dialog: bordered, with optional letter-by-letter reveal of one line.
function DialogBox({ theme, lines, animate = false, style }) {
  const [revealed, setRevealed] = useState(animate ? '' : null);
  useEffect(() => {
    if (!animate || !lines || lines.length === 0) return;
    const full = lines.join('\n');
    let i = 0;
    setRevealed('');
    const id = setInterval(() => {
      i += 1;
      setRevealed(full.slice(0, i));
      if (i >= full.length) clearInterval(id);
    }, 28);
    return () => clearInterval(id);
  }, [animate, JSON.stringify(lines)]);

  const text = animate ? revealed ?? '' : (lines || []).join('\n');
  return (
    <div
      style={{
        background: theme.win,
        border: `2px solid ${theme.border}`,
        position: 'relative',
        ...style,
      }}
    >
      <div style={{ padding: 3 }}>
        <div
          style={{
            border: `2px solid ${theme.border}`,
            padding: '14px 18px 18px',
            fontFamily: 'Galmuri11, "DotGothic16", monospace',
            fontSize: 14,
            lineHeight: 1.55,
            color: theme.text,
            whiteSpace: 'pre-wrap',
            minHeight: 56,
          }}
        >
          {text}
          {animate && revealed !== null && revealed.length < (lines || []).join('\n').length && (
            <span style={{ color: theme.gold, animation: 'luida-blink 1s steps(1) infinite' }}>▌</span>
          )}
          {(!animate || (revealed !== null && revealed.length >= (lines || []).join('\n').length)) && (
            <span
              style={{
                position: 'absolute',
                right: 12,
                bottom: 10,
                color: theme.gold,
                fontSize: 12,
                animation: 'luida-blink 1s steps(1) infinite',
              }}
            >
              ▼
            </span>
          )}
        </div>
      </div>
    </div>
  );
}

// Cursor-driven menu list. items: [{label, hint, value, danger}].
function MenuList({ theme, items, value, onChange, onSelect, dense, mono }) {
  const idx = Math.max(
    0,
    items.findIndex((it) => it.value === value),
  );
  return (
    <div role="menu" style={{ display: 'flex', flexDirection: 'column' }}>
      {items.map((it, i) => {
        const active = i === idx;
        return (
          <button
            key={it.value ?? i}
            type="button"
            onClick={() => {
              onChange && onChange(it.value);
              onSelect && onSelect(it.value);
            }}
            onMouseEnter={() => onChange && onChange(it.value)}
            style={{
              all: 'unset',
              cursor: 'default',
              display: 'flex',
              alignItems: 'center',
              gap: 10,
              padding: dense ? '4px 6px' : '6px 8px',
              fontFamily: mono ? '"DotGothic16", monospace' : 'Galmuri11, "DotGothic16", monospace',
              fontSize: dense ? 13 : 14,
              color: active ? theme.text : theme.dim,
              background: 'transparent',
              letterSpacing: mono ? 0 : 0,
            }}
          >
            <span style={{ width: 14, color: active ? theme.gold : 'transparent', flexShrink: 0 }}>▶</span>
            <span style={{ flex: 1, color: it.danger ? theme.red : 'inherit' }}>{it.label}</span>
            {it.hint && (
              <span style={{ color: theme.dim, fontFamily: '"DotGothic16", monospace', fontSize: 11 }}>
                {it.hint}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}

// Status badge — small filled rectangle with color coding.
function Badge({ theme, kind, label, size = 'sm' }) {
  const palette = {
    running: theme.blue,
    reviewing: theme.gold,
    needs_approval: theme.pink,
    pr_ready: theme.green,
    pending: theme.dim,
    completed: theme.green,
    failed: theme.red,
    aborted: theme.dim,
    busy: theme.gold,
    idle: theme.green,
    offline: theme.dim,
    main: theme.pink,
    worker: theme.blue,
    brain: theme.gold,
    info: theme.dim,
  };
  const bg = palette[kind] || theme.dim;
  const fg = '#0A0A0A';
  return (
    <span
      style={{
        display: 'inline-block',
        background: bg,
        color: fg,
        fontFamily: '"DotGothic16", monospace',
        fontSize: size === 'lg' ? 12 : 10,
        letterSpacing: 1,
        padding: size === 'lg' ? '3px 8px' : '2px 6px',
        textTransform: 'uppercase',
        lineHeight: 1.2,
        whiteSpace: 'nowrap',
      }}
    >
      {label}
    </span>
  );
}

// Pixel-block HP/MP bar — 20 cells, each filled or empty.
function StatusBar({ theme, label, current, max, color, width, segments = 20 }) {
  const filled = Math.round((current / max) * segments);
  const pct = current / max;
  const auto =
    pct < 0.25 ? theme.red : pct < 0.55 ? theme.yellow : theme.green;
  const fill = color === 'mp' ? theme.blue : color || auto;
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontFamily: '"DotGothic16", monospace' }}>
      {label && (
        <span style={{ color: theme.gold, fontSize: 11, letterSpacing: 1, width: 22, flexShrink: 0 }}>
          {label}
        </span>
      )}
      <div
        style={{
          display: 'flex',
          gap: 1,
          background: '#000',
          padding: 2,
          border: `1px solid ${theme.dim}66`,
          flex: width ? 0 : 1,
          width: width || 'auto',
        }}
      >
        {Array.from({ length: segments }).map((_, i) => (
          <div
            key={i}
            style={{
              flex: 1,
              height: 8,
              background: i < filled ? fill : '#0a0a1a',
              minWidth: 3,
            }}
          />
        ))}
      </div>
      <span style={{ color: theme.text, fontSize: 11, minWidth: 52, textAlign: 'right' }}>
        {current}/{max}
      </span>
    </div>
  );
}

// Thin progress strip for quest rows.
function ProgressStrip({ theme, pct, status }) {
  const color =
    status === 'failed' ? theme.red :
    status === 'aborted' ? theme.dim :
    status === 'completed' || status === 'pr_ready' ? theme.green :
    status === 'needs_approval' ? theme.pink :
    status === 'reviewing' ? theme.gold :
    theme.blue;
  const segs = 14;
  const filled = Math.round((pct / 100) * segs);
  return (
    <div style={{ display: 'flex', gap: 1, background: '#000', padding: 2, border: `1px solid ${theme.dim}55` }}>
      {Array.from({ length: segs }).map((_, i) => (
        <div
          key={i}
          style={{
            flex: 1,
            height: 6,
            background: i < filled ? color : '#0a0a1a',
            minWidth: 4,
          }}
        />
      ))}
    </div>
  );
}

// Pill button — retro chunky, no rounding.
function PixelButton({ theme, children, onClick, variant = 'default', size = 'md', style, full }) {
  const v = {
    default: { bg: theme.winAlt, fg: theme.text, border: theme.border },
    primary: { bg: theme.gold, fg: '#080814', border: theme.border },
    danger: { bg: theme.red, fg: '#080814', border: theme.border },
    success: { bg: theme.green, fg: '#080814', border: theme.border },
    ghost: { bg: 'transparent', fg: theme.dim, border: theme.dim },
  }[variant];
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        all: 'unset',
        cursor: 'default',
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 6,
        background: v.bg,
        color: v.fg,
        border: `2px solid ${v.border}`,
        padding: size === 'sm' ? '3px 10px' : size === 'lg' ? '10px 22px' : '6px 14px',
        fontFamily: 'Galmuri11, "DotGothic16", monospace',
        fontSize: size === 'sm' ? 12 : size === 'lg' ? 16 : 14,
        letterSpacing: 1,
        textTransform: 'uppercase',
        width: full ? '100%' : 'auto',
        boxShadow: `3px 3px 0 0 ${theme.bg}`,
        ...style,
      }}
    >
      {children}
    </button>
  );
}

export { Window, DialogBox, MenuList, Badge, StatusBar, ProgressStrip, PixelButton };
