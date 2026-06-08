//! 협조적 취소 토큰 — TUI 종료 시 실행 중인 worker(외부 CLI 자식 프로세스)를
//! 즉시 정리해 고아 프로세스를 막는다.
//!
//! 설계: 토큰은 **런타임 factory 에 주입**되어 런타임 계층에만 존재한다(시그니처 전파 최소화).
//! 런타임은 자식 spawn 직후 PID 를 등록하고, 스트림 루프에서 취소 플래그를 확인한다.
//! `cancel()` 은 플래그를 세우고 등록된 PID 를 SIGKILL → 블록된 자식도 즉시 종료된다.
//! 취소로 끝난 실행은 `AgentOutcome.cancelled=true` 로 표시되어 상위(settle_outcome)에서
//! 'failed' 가 아닌 '중단(interrupted)'으로 처리된다.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// 공유 취소 신호 + 현재 자식 프로세스 PID. Clone 은 같은 신호를 가리킨다(Arc).
#[derive(Clone, Default, Debug)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
    /// 현재 실행 중인 자식 PID(0=없음). 취소 시 이 PID 를 kill 한다.
    child_pid: Arc<AtomicU32>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// 취소 요청: 플래그를 세우고, 등록된 자식이 있으면 즉시 SIGKILL.
    /// (PID 재사용 TOCTOU 는 clear_child + SIGKILL 즉시성으로 실질적 위험이 낮다.)
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let pid = self.child_pid.load(Ordering::SeqCst);
        if pid != 0 {
            kill_pid(pid);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// 런타임이 자식 spawn 직후 호출 — 취소 시 kill 대상 PID 등록.
    /// 등록 시점에 이미 취소됐다면 즉시 kill 한다(취소→spawn 경합 보호).
    pub fn register_child(&self, pid: u32) {
        self.child_pid.store(pid, Ordering::SeqCst);
        if self.is_cancelled() {
            kill_pid(pid);
        }
    }

    /// 자식 reap 후 호출 — 오래된 PID 로 무관한 프로세스를 죽이지 않도록 해제.
    pub fn clear_child(&self) {
        self.child_pid.store(0, Ordering::SeqCst);
    }

    /// 현재 등록된 자식 PID(0=없음). 테스트·진단용.
    pub fn registered_pid(&self) -> u32 {
        self.child_pid.load(Ordering::SeqCst)
    }
}

/// PID 에 SIGKILL (unix). 비-unix 는 no-op.
fn kill_pid(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// PID 가 살아있는지(시그널 0 으로 존재 확인). 재조정(reconcile)에서 고아 판정에 쓴다.
/// unix 외에서는 보수적으로 true(살아있다고 가정 → 함부로 중단 처리하지 않음).
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        if pid == 0 {
            return false;
        }
        // kill(pid, 0): 0=존재, ESRCH=없음, EPERM=존재하지만 권한없음(=살아있음).
        let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if r == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

/// PID 프로세스의 시작 시각(epoch ms). PID 재사용 구분용 — 같은 PID 라도 시작시각이 다르면
/// 재사용된 다른 프로세스다. 조회 불가/미지원 플랫폼은 None.
pub fn process_start_time(pid: u32) -> Option<i64> {
    #[cfg(target_os = "macos")]
    {
        // libproc: proc_pidinfo(PROC_PIDTBSDINFO) → proc_bsdinfo.pbi_start_tvsec/tvusec.
        if pid == 0 {
            return None;
        }
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let rc = unsafe {
            libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDTBSDINFO,
                0,
                &mut info as *mut _ as *mut libc::c_void,
                size,
            )
        };
        // 반환값 = 기록된 바이트 수. 구조체 크기만큼 채워졌을 때만 유효.
        if rc < size {
            return None;
        }
        Some(info.pbi_start_tvsec as i64 * 1000 + info.pbi_start_tvusec as i64 / 1000)
    }
    #[cfg(target_os = "linux")]
    {
        // /proc/<pid>/stat 의 22번째 필드(starttime, clock ticks since boot).
        // comm(2번 필드)에 공백/괄호가 있을 수 있어 마지막 ')' 이후부터 센다.
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let after = stat.rsplit_once(") ")?.1; // 3번 필드(state)부터
        let ticks: i64 = after.split_whitespace().nth(19)?.parse().ok()?; // 22번 필드
        Some(ticks)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

/// runner 가 (PID 생존 + 시작시각 일치로) 살아있는지. 시작시각을 모르면 PID 생존만으로 판단(보수적).
/// → PID 재사용으로 죽은 runner 를 살아있다고 오판하지 않게 한다.
pub fn runner_alive(pid: u32, started_at: Option<i64>) -> bool {
    if !pid_alive(pid) {
        return false;
    }
    match (started_at, process_start_time(pid)) {
        // 둘 다 알면 시작시각이 같아야 같은 프로세스(=살아있음). 다르면 재사용된 PID(=죽음).
        (Some(stored), Some(current)) => stored == current,
        // 시작시각 불명 → PID 생존만으로 살아있다고 본다(함부로 중단하지 않음).
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_sets_flag() {
        let t = CancelToken::new();
        assert!(!t.is_cancelled());
        t.cancel();
        assert!(t.is_cancelled());
    }

    #[test]
    fn clone_shares_signal() {
        let t = CancelToken::new();
        let t2 = t.clone();
        t.cancel();
        assert!(t2.is_cancelled()); // 같은 Arc 신호
    }

    #[test]
    fn register_and_clear_child_pid() {
        let t = CancelToken::new();
        // 등록 전 취소는 자식 kill 없이 플래그만.
        t.register_child(0); // 0 은 무시
        t.clear_child();
        assert!(!t.is_cancelled());
    }

    #[test]
    fn pid_alive_self_is_true_zero_is_false() {
        let me = std::process::id();
        assert!(pid_alive(me));
        assert!(!pid_alive(0));
    }

    #[test]
    fn pid_alive_dead_pid_is_false() {
        // 매우 큰 PID 는 거의 확실히 존재하지 않음.
        assert!(!pid_alive(4_000_000_000));
    }

    #[test]
    fn process_start_time_self_stable() {
        let me = std::process::id();
        let a = process_start_time(me);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            assert!(a.is_some(), "지원 플랫폼에서 자기 시작시각은 Some");
            assert_eq!(a, process_start_time(me), "시작시각은 안정적");
        }
        let _ = a; // 비지원 플랫폼은 None 허용
    }

    #[test]
    fn runner_alive_distinguishes_pid_reuse() {
        let me = std::process::id();
        let st = process_start_time(me);
        // 나 자신 + 내 시작시각 → 살아있음.
        assert!(runner_alive(me, st));
        // 죽은 pid → 죽음.
        assert!(!runner_alive(4_000_000_000, Some(123)));
        // 시작시각 불명 → PID 생존만으로 보수적 판단(살아있음).
        assert!(runner_alive(me, None));
        // 지원 플랫폼: 시작시각이 어긋나면(=PID 재사용 모사) 죽은 것으로 본다.
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        assert!(!runner_alive(me, Some(st.unwrap() + 999_999)));
    }
}
