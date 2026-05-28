#!/usr/bin/env bun
// 스펙 markdown들을 단일 HTML 보고서로 발행.
//   사용: bun run scripts/build-report.ts
//   출력: docs/reports/luida-v2-spec.html (self-contained, 브라우저로 열어 검토)
//
// markdown은 base64로 임베드하고 marked.js(CDN, 고정 버전)로 클라이언트 렌더.
// TOC는 렌더 후 heading에서 자동 생성. DQ 풍 다크 테마.

import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';

const ROOT = resolve(import.meta.dir, '..');

type Doc = { id: string; title: string; path: string };

const DOCS: Doc[] = [
  {
    id: 'v2',
    title: 'v2 Standalone Architecture',
    path: 'docs/v2-standalone.md',
  },
  {
    id: 'adr1',
    title: 'ADR-0001 — Rust vs TypeScript',
    path: 'docs/adr/0001-rust-vs-typescript-for-v2.md',
  },
];

function b64(s: string): string {
  return Buffer.from(s, 'utf8').toString('base64');
}

const sections = DOCS.map((d) => {
  const full = join(ROOT, d.path);
  if (!existsSync(full)) {
    throw new Error(`문서를 찾을 수 없음: ${d.path}`);
  }
  const md = readFileSync(full, 'utf8');
  return {
    id: d.id,
    title: d.title,
    path: d.path,
    b64: b64(md),
  };
});

const generatedAt = new Date().toISOString();

const html = `<!doctype html>
<html lang="ko">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>🍺 Luida 스펙 보고서</title>
<script src="https://cdn.jsdelivr.net/npm/marked@14.1.3/marked.min.js"></script>
<style>
  :root {
    --bg: #050a14; --panel: #0b1424; --border: #1e2d44;
    --text: #e6edf7; --dim: #8aa0c0; --gold: #FCD34D; --accent: #60A5FA;
    --green: #4ADE80; --pink: #F472B6; --code: #0a1322;
  }
  * { box-sizing: border-box; }
  body {
    margin: 0; background: var(--bg); color: var(--text);
    font-family: -apple-system, "Apple SD Gothic Neo", "Pretendard", system-ui, sans-serif;
    line-height: 1.7; font-size: 15px;
  }
  .layout { display: flex; min-height: 100vh; }
  nav {
    width: 300px; flex-shrink: 0; background: var(--panel);
    border-right: 1px solid var(--border); padding: 24px 18px;
    position: sticky; top: 0; height: 100vh; overflow-y: auto;
  }
  nav h1 { font-size: 18px; color: var(--gold); margin: 0 0 4px; }
  nav .meta { font-size: 11px; color: var(--dim); margin-bottom: 20px; }
  nav .doc-title {
    font-size: 13px; color: var(--accent); text-transform: uppercase;
    letter-spacing: 1px; margin: 18px 0 6px; font-weight: 700;
  }
  nav a {
    display: block; color: var(--dim); text-decoration: none;
    font-size: 13px; padding: 3px 0 3px 10px; border-left: 2px solid transparent;
  }
  nav a:hover { color: var(--text); border-left-color: var(--gold); }
  nav a.h3 { padding-left: 22px; font-size: 12px; }
  main { flex: 1; padding: 40px 56px 120px; max-width: 980px; }
  .doc { margin-bottom: 80px; padding-bottom: 40px; border-bottom: 2px dashed var(--border); }
  .doc-src { font-size: 12px; color: var(--dim); font-family: monospace; margin-bottom: 8px; }
  h1, h2, h3, h4 { line-height: 1.3; scroll-margin-top: 20px; }
  h1 { color: var(--gold); border-bottom: 2px solid var(--border); padding-bottom: 10px; }
  h2 { color: var(--accent); margin-top: 40px; border-bottom: 1px solid var(--border); padding-bottom: 6px; }
  h3 { color: var(--green); margin-top: 28px; }
  h4 { color: var(--pink); }
  a { color: var(--accent); }
  code {
    background: var(--code); padding: 2px 6px; border-radius: 3px;
    font-size: 13px; color: #ffd9a0; font-family: "SF Mono", Menlo, monospace;
  }
  pre {
    background: var(--code); border: 1px solid var(--border); border-radius: 6px;
    padding: 16px; overflow-x: auto;
  }
  pre code { background: none; padding: 0; color: #cde3ff; }
  table { border-collapse: collapse; width: 100%; margin: 16px 0; font-size: 14px; }
  th, td { border: 1px solid var(--border); padding: 7px 11px; text-align: left; vertical-align: top; }
  th { background: var(--panel); color: var(--gold); }
  tr:nth-child(even) td { background: #0a1220; }
  blockquote {
    border-left: 3px solid var(--gold); margin: 16px 0; padding: 4px 16px;
    background: #0c1626; color: var(--dim);
  }
  hr { border: none; border-top: 1px solid var(--border); margin: 32px 0; }
  .badge {
    display: inline-block; background: var(--gold); color: #081018;
    font-size: 11px; font-weight: 700; padding: 2px 8px; border-radius: 3px;
    letter-spacing: 1px;
  }
</style>
</head>
<body>
<div class="layout">
  <nav>
    <h1>🍺 Luida 스펙</h1>
    <div class="meta">발행: ${generatedAt}<br/><span class="badge">v2 Rust 확정</span></div>
    <div id="toc"></div>
  </nav>
  <main id="content"></main>
</div>

<script id="payload" type="application/json">${JSON.stringify(sections)}</script>
<script>
  function decodeB64Utf8(b64) {
    const bin = atob(b64);
    const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
    return new TextDecoder('utf-8').decode(bytes);
  }
  function slugify(s, used) {
    let base = s.toLowerCase().replace(/[^\\w가-힣]+/g, '-').replace(/^-+|-+$/g, '') || 'h';
    let id = base, n = 1;
    while (used.has(id)) { id = base + '-' + (++n); }
    used.add(id); return id;
  }

  const sections = JSON.parse(document.getElementById('payload').textContent);
  const content = document.getElementById('content');
  const toc = document.getElementById('toc');
  const usedIds = new Set();

  marked.setOptions({ gfm: true, breaks: false });

  sections.forEach((sec) => {
    const md = decodeB64Utf8(sec.b64);
    const docEl = document.createElement('div');
    docEl.className = 'doc';

    const src = document.createElement('div');
    src.className = 'doc-src';
    src.textContent = sec.path;
    docEl.appendChild(src);

    const body = document.createElement('div');
    body.innerHTML = marked.parse(md);
    docEl.appendChild(body);
    content.appendChild(docEl);

    // TOC: 이 문서의 h1/h2/h3에 id 부여 + 링크
    const docTitle = document.createElement('div');
    docTitle.className = 'doc-title';
    docTitle.textContent = sec.title;
    toc.appendChild(docTitle);

    body.querySelectorAll('h1, h2, h3').forEach((h) => {
      const id = slugify(sec.id + '-' + h.textContent, usedIds);
      h.id = id;
      const a = document.createElement('a');
      a.href = '#' + id;
      a.textContent = h.textContent;
      if (h.tagName === 'H3') a.className = 'h3';
      if (h.tagName === 'H1') a.style.color = 'var(--gold)';
      toc.appendChild(a);
    });
  });
</script>
</body>
</html>
`;

const outDir = join(ROOT, 'docs', 'reports');
if (!existsSync(outDir)) mkdirSync(outDir, { recursive: true });
const outPath = join(outDir, 'luida-v2-spec.html');
writeFileSync(outPath, html);
console.log(`📄 보고서 발행 완료: ${outPath}`);
console.log(`   문서 ${sections.length}건 · ${(html.length / 1024).toFixed(0)}KB`);
console.log(`   브라우저로 열기: open ${outPath}`);
void dirname;
