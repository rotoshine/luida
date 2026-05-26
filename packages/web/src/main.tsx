// @ts-nocheck
// Vite entry — mounts App into #root.
//
// app.tsx의 마지막 줄에 있던 ReactDOM.createRoot 호출을
// React 18 표준 패턴으로 옮김 (react-dom/client에서 import).

import React, { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './app';
import type { TavernSnapshot } from './live';
import { LiveProvider } from './live';

function Root() {
  return (
    <LiveProvider>
      <App />
    </LiveProvider>
  );
}

const container = document.getElementById('root');
if (!container) throw new Error('#root not found');
createRoot(container).render(<Root />);
