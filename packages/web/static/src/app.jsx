// Root app: tab nav, tweaks panel wiring, toast plumbing.

const { useState: useState_A, useEffect: useEffect_A } = React;

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "theme": "navy",
  "lang": "ko",
  "density": "regular",
  "fontScale": 100
}/*EDITMODE-END*/;

function TabNav({ theme, lang, current, onChange }) {
  const i18n = window.LUIDA_I18N[lang];
  const tabs = [
    { key: 'dashboard', label: i18n.nav[3], hint: '메인 주점 전경' },
    { key: 'moodboard', label: i18n.nav[0], hint: '컨셉 · 원칙' },
    { key: 'tokens', label: i18n.nav[1], hint: '컬러 · 타입 · 간격' },
    { key: 'components', label: i18n.nav[2], hint: '공용 컴포넌트' },
  ];
  return (
    <div style={{ display: 'flex', gap: 8, padding: '14px 20px 0', flexWrap: 'wrap' }}>
      {tabs.map((t) => {
        const active = t.key === current;
        return (
          <button
            key={t.key}
            type="button"
            onClick={() => onChange(t.key)}
            style={{
              all: 'unset',
              cursor: 'default',
              padding: '6px 14px',
              fontFamily: 'Galmuri11, "DotGothic16", monospace',
              fontSize: 13,
              letterSpacing: 1,
              textTransform: 'uppercase',
              color: active ? '#080814' : theme.text,
              background: active ? theme.gold : theme.win,
              border: `2px solid ${active ? theme.gold : theme.border}`,
              boxShadow: `3px 3px 0 0 ${theme.bg}`,
              display: 'flex',
              alignItems: 'center',
              gap: 8,
            }}
          >
            <span style={{ color: active ? '#080814' : theme.gold }}>{active ? '▶' : '·'}</span>
            <span>{t.label}</span>
            <span
              style={{
                fontFamily: '"DotGothic16", monospace',
                fontSize: 10,
                color: active ? '#080814aa' : theme.dim,
                letterSpacing: 1,
              }}
            >
              {t.hint}
            </span>
          </button>
        );
      })}
    </div>
  );
}

function App() {
  const [t, setTweak] = useTweaks(TWEAK_DEFAULTS);
  const theme = window.LUIDA_TOKENS.themes[t.theme] || window.LUIDA_TOKENS.themes.navy;
  const [current, setCurrent] = useState_A('dashboard');
  const [toast, setToast] = useState_A(null);

  useEffect_A(() => {
    if (!toast) return;
    const id = setTimeout(() => setToast(null), 4500);
    return () => clearTimeout(id);
  }, [toast]);

  // Welcome toast on first load
  useEffect_A(() => {
    const id = setTimeout(() => {
      setToast('🍺 주점에 신규 의뢰 #143이 admin에게 전달되었어요.');
    }, 1400);
    return () => clearTimeout(id);
  }, []);

  const showToast = (m) => setToast(m);

  return (
    <div
      style={{
        background: theme.bg,
        minHeight: '100vh',
        color: theme.text,
        fontSize: `${t.fontScale}%`,
        fontFamily: 'Galmuri11, "DotGothic16", monospace',
      }}
      data-screen-label={current === 'dashboard' ? '01 Dashboard' : current === 'moodboard' ? '02 Moodboard' : current === 'tokens' ? '03 Tokens' : '04 Components'}
    >
      <TabNav theme={theme} lang={t.lang} current={current} onChange={setCurrent} />

      <div
        style={{
          padding: '20px 24px 80px',
          maxWidth: 1480,
          margin: '0 auto',
        }}
      >
        {current === 'dashboard' && (
          <DashboardTab theme={theme} lang={t.lang} density={t.density} onShowToast={showToast} />
        )}
        {current === 'moodboard' && <MoodboardTab theme={theme} lang={t.lang} />}
        {current === 'tokens' && <TokensTab theme={theme} lang={t.lang} />}
        {current === 'components' && <ComponentsTab theme={theme} lang={t.lang} />}
      </div>

      {toast && <Toast theme={theme} message={toast} onDismiss={() => setToast(null)} />}

      <TweaksPanel title="Tweaks">
        <TweakSection label="Theme" />
        <TweakSelect
          label="컬러 테마"
          value={t.theme}
          options={[
            { value: 'navy', label: '블랙 (정통)' },
            { value: 'slate', label: '슬레이트' },
            { value: 'purple', label: '바이올렛' },
            { value: 'forest', label: '심야 숲' },
          ]}
          onChange={(v) => setTweak('theme', v)}
        />
        <TweakRadio
          label="Language"
          value={t.lang}
          options={['ko', 'en']}
          onChange={(v) => setTweak('lang', v)}
        />
        <TweakSection label="Layout" />
        <TweakRadio
          label="Density"
          value={t.density}
          options={['compact', 'regular', 'comfy']}
          onChange={(v) => setTweak('density', v)}
        />
        <TweakSlider
          label="Font scale"
          value={t.fontScale}
          min={85}
          max={130}
          step={5}
          unit="%"
          onChange={(v) => setTweak('fontScale', v)}
        />
        <TweakSection label="Actions" />
        <TweakButton onClick={() => setToast('💀 forge가 쓰러졌습니다 — Bun 1.2 typecheck 실패')}>
          토스트 미리보기 (실패)
        </TweakButton>
        <TweakButton onClick={() => setToast('💡 새 패턴 후보 — lantern → kontrol 캐시 무효화 (신뢰도 9/10)')}>
          토스트 미리보기 (제안)
        </TweakButton>
      </TweaksPanel>
    </div>
  );
}

ReactDOM.createRoot(document.getElementById('root')).render(<App />);
