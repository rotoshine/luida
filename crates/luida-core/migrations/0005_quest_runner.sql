-- Luida v2 — quest runner lease (재시작 시 고아/중단 재조정용).
-- 어느 프로세스가 이 quest 를 running 으로 돌리고 있는지 기록. 시작 시 죽은 runner 의
-- running quest 는 'pending' 으로 되돌려(중단) 이어받기 가능하게 한다.
-- status CHECK 는 건드리지 않는다(중단은 기존 'pending' + quest_interrupted 이벤트로 표현).

ALTER TABLE quests ADD COLUMN runner_pid INTEGER;
ALTER TABLE quests ADD COLUMN runner_machine TEXT;
-- runner 프로세스의 시작 시각. PID 재사용 구분용 — 같은 PID 라도 시작시각이 다르면
-- 다른(=죽은) 프로세스다. 이게 없으면 PID 재사용으로 죽은 runner 를 살아있다고 오판해 quest 가
-- 'running' 으로 영영 묶일 수 있다.
-- 단위는 플랫폼 의존(macOS=epoch ms, Linux=boot 이후 clock ticks)이며, 같은 머신에서
-- set_runner 가 찍은 값과 process_start_time() 결과의 '동일성' 비교에만 쓴다 → 절대 단위는 무의미.
ALTER TABLE quests ADD COLUMN runner_started_at INTEGER;
