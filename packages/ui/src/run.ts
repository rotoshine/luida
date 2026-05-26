import { render } from 'ink';
import { createElement } from 'react';
import { App, type AppProps } from './App';

/**
 * 대시보드를 띄운다. `luida ui` 명령에서 호출.
 *
 * 견고성:
 *  - stdin이 TTY가 아니면(파이프, nohup 등) 즉시 에러로 안내
 *  - SIGTERM 시 명시적 unmount로 alternate screen 복구 보장
 */
export async function runUi(opts: AppProps = {}): Promise<void> {
  if (!process.stdin.isTTY) {
    throw new Error(
      'luida ui는 TTY 환경에서만 동작합니다 (파이프/nohup 환경 미지원). cmux pane 안에서 실행해주세요.',
    );
  }

  const instance = render(createElement(App, opts), {
    exitOnCtrlC: true,
    patchConsole: false,
  });

  const onTerm = (): void => {
    try {
      instance.unmount();
    } catch {
      // ignore
    }
  };
  process.once('SIGTERM', onTerm);

  try {
    await instance.waitUntilExit();
  } finally {
    process.off('SIGTERM', onTerm);
  }
}
