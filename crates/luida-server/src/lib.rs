//! luida-server — Rust core daemon의 HTTP/SSE API (ADR-0002 브리지).
//!
//! 클라이언트(Tauri GUI / 향후 Ink TUI)가 이 API의 thin client.
//! 현재: /api/health, /api/snapshot(읽기), /api/stream(SSE). command는 후속.
//!
//! rusqlite Connection은 !Sync라 Arc<Mutex<Connection>>로 공유.

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::Result;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use luida_core::{open_ready, CampaignRepo, Connection, InmailRepo, ProjectRepo, QuestRepo};
use luida_planner::{plan_campaign, run_campaign};
use luida_runtimes::make_factory;
use luida_sidecar::{make_worktree, resume_quest, triage_escalation};
use serde::Deserialize;
use serde_json::{json, Value};

/// 공유 상태 — 읽기용 DB connection(Mutex) + 명령용 db 경로.
/// 명령(plan/run/resume)은 오래 걸리므로 read conn 의 Mutex 를 잡지 않고
/// db_path 로 별도 connection 을 열어 실행한다(WAL 동시 read/write).
#[derive(Clone)]
pub struct AppState {
    conn: Arc<Mutex<Connection>>,
    db_path: PathBuf,
}

/// Mutex poisoning을 복구해 잠금 (한 핸들러의 패닉이 서버 전체를 죽이지 않게). review C3.
fn lock_recover(m: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 동기 SQLite 쿼리를 blocking 풀에서 실행 (tokio 워커 블로킹 방지). review C2.
async fn snapshot_blocking(state: AppState) -> Value {
    tokio::task::spawn_blocking(move || {
        let conn = lock_recover(&state.conn);
        snapshot_json(&conn).unwrap_or_else(|e| json!({ "error": e.to_string() }))
    })
    .await
    .unwrap_or_else(|e| json!({ "error": format!("join error: {e}") }))
}

/// 라우터 구성. 테스트는 이걸 직접 oneshot 호출.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/health", get(health))
        .route("/api/snapshot", get(snapshot))
        .route("/api/stream", get(stream))
        .route("/api/projects", post(create_project))
        .route("/api/campaigns/plan", post(plan_http))
        .route("/api/campaigns/{id}/run", post(run_http))
        .route("/api/quests/{id}/resume", post(resume_http))
        .route("/api/quests/{id}/triage", post(triage_http))
        .with_state(state)
}

/// 웹 대시보드 (단일 HTML, 바이너리에 임베드).
async fn index() -> impl IntoResponse {
    Html(include_str!("dashboard.html"))
}

async fn health() -> &'static str {
    "OK"
}

/// 모험지 등록 요청 본문.
#[derive(Deserialize)]
struct NewProjectReq {
    name: String,
    repo_path: String,
    base_branch: Option<String>,
    description: Option<String>,
}

/// POST /api/projects — 모험지 등록(upsert). command API의 첫 쓰기 엔드포인트.
async fn create_project(
    State(state): State<AppState>,
    Json(req): Json<NewProjectReq>,
) -> impl IntoResponse {
    if req.name.trim().is_empty() || req.repo_path.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "name·repo_path는 필수" })),
        );
    }
    let name = req.name.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = lock_recover(&state.conn);
        ProjectRepo::new(&conn).add(
            &req.name,
            &req.repo_path,
            req.base_branch.as_deref().unwrap_or("main"),
            req.description.as_deref(),
        )
    })
    .await;
    match result {
        Ok(Ok(())) => (StatusCode::CREATED, Json(json!({ "ok": true, "name": name }))),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("join error: {e}") })),
        ),
    }
}

/// spawn_blocking 의 `Result<Result<Value>, JoinError>` 를 HTTP 응답으로 변환.
fn json_result(
    r: std::result::Result<Result<Value>, tokio::task::JoinError>,
) -> (StatusCode, Json<Value>) {
    match r {
        Ok(Ok(v)) => (StatusCode::OK, Json(v)),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("join error: {e}") })),
        ),
    }
}

#[derive(Deserialize)]
struct PlanReq {
    prompt: String,
}

/// POST /api/campaigns/plan — 사용자 프롬프트 → 원정 계획.
async fn plan_http(State(state): State<AppState>, Json(req): Json<PlanReq>) -> impl IntoResponse {
    let db = state.db_path.clone();
    let r = tokio::task::spawn_blocking(move || -> Result<Value> {
        let (mut conn, cfg) = open_ready(&db)?;
        let cid = plan_campaign(&mut conn, &cfg, &req.prompt, make_factory())?;
        Ok(json!({ "campaign_id": cid }))
    })
    .await;
    json_result(r)
}

/// POST /api/campaigns/{id}/run — 원정 실행(의존성 순, 완료까지 블로킹).
async fn run_http(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let db = state.db_path.clone();
    let r = tokio::task::spawn_blocking(move || -> Result<Value> {
        let (mut conn, cfg) = open_ready(&db)?;
        let report = run_campaign(&mut conn, &cfg, id, make_worktree().as_ref(), make_factory())?;
        Ok(json!({
            "completed": report.completed.len(),
            "needs_input": report.needs_input.len(),
            "failed": report.failed.len(),
            "triggered": report.triggered,
            "all_completed": report.all_completed,
        }))
    })
    .await;
    json_result(r)
}

#[derive(Deserialize)]
struct ResumeReq {
    answer: String,
}

/// POST /api/quests/{id}/resume — needs_input 모험을 답변으로 재개.
async fn resume_http(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<ResumeReq>,
) -> impl IntoResponse {
    let db = state.db_path.clone();
    let r = tokio::task::spawn_blocking(move || -> Result<Value> {
        let (mut conn, cfg) = open_ready(&db)?;
        let out = resume_quest(&mut conn, &cfg, id, &req.answer, make_factory())?;
        Ok(json!({ "outcome": format!("{out:?}") }))
    })
    .await;
    json_result(r)
}

/// POST /api/quests/{id}/triage — escalation 분류(자동 해소 가능 여부).
async fn triage_http(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let db = state.db_path.clone();
    let r = tokio::task::spawn_blocking(move || -> Result<Value> {
        let (mut conn, cfg) = open_ready(&db)?;
        let d = triage_escalation(&mut conn, &cfg, id, make_factory())?;
        Ok(json!({ "ask_user": d.ask_user, "auto_answer": d.auto_answer, "reason": d.reason }))
    })
    .await;
    json_result(r)
}

/// tavern.db 스냅샷 JSON.
fn snapshot_json(conn: &Connection) -> Result<Value> {
    let projects = ProjectRepo::new(conn).list()?;
    let campaigns = CampaignRepo::new(conn).list_active()?;
    let quests = QuestRepo::new(conn).list_active()?;
    let inmail = InmailRepo::new(conn).tail(50)?;
    Ok(json!({
        "projects": projects,
        "campaigns": campaigns,
        "quests": quests,
        "inmail": inmail,
        "taken_at": luida_core::now_ms(),
    }))
}

async fn snapshot(State(state): State<AppState>) -> impl IntoResponse {
    Json(snapshot_blocking(state).await)
}

/// SSE — 1초 주기 스냅샷 스트림.
/// 각 tick의 DB 접근은 spawn_blocking으로 async 워커를 막지 않음.
/// 클라이언트 연결 종료 시 yield 실패로 stream future가 drop되어 루프 정리됨.
async fn stream(
    State(state): State<AppState>,
) -> Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>> {
    let s = async_stream::stream! {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        loop {
            ticker.tick().await;
            let payload = snapshot_blocking(state.clone()).await;
            yield Ok(Event::default().data(payload.to_string()));
        }
    };
    Sse::new(s).keep_alive(KeepAlive::default())
}

/// 서버 실행 (127.0.0.1:port). luida server start에서 호출.
pub async fn serve(port: u16, conn: Connection, db_path: PathBuf) -> Result<()> {
    let state = AppState {
        conn: Arc::new(Mutex::new(conn)),
        db_path,
    };
    let app = build_router(state);
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("🛰  luida-server listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use luida_core::{migrate, open_memory, NewCampaign, NewQuest, ProjectRepo};
    use tower::ServiceExt;

    fn seeded_state() -> AppState {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        {
            let p = ProjectRepo::new(&conn);
            p.add("agora", "/a", "main", None).unwrap();
            let cid = CampaignRepo::new(&conn)
                .insert(NewCampaign {
                    title: "t",
                    prompt: "p",
                    plan_json: "{}",
                    status: "running",
                })
                .unwrap();
            QuestRepo::new(&conn)
                .insert(NewQuest {
                    campaign_id: Some(cid),
                    project: "agora",
                    brief: "작업",
                    branch: None,
                    status: "running",
                    depends_on_quest_id: None,
                    source_inmail_id: None,
                })
                .unwrap();
        }
        AppState {
            conn: Arc::new(Mutex::new(conn)),
            db_path: std::path::PathBuf::from(":memory:"),
        }
    }

    #[tokio::test]
    async fn health_ok() {
        let app = build_router(seeded_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"OK");
    }

    #[tokio::test]
    async fn snapshot_returns_seeded_data() {
        let app = build_router(seeded_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/snapshot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["projects"].as_array().unwrap().len(), 1);
        assert_eq!(v["campaigns"].as_array().unwrap().len(), 1);
        assert_eq!(v["quests"].as_array().unwrap().len(), 1);
        assert!(v["taken_at"].as_i64().unwrap() > 0);
    }

    #[tokio::test]
    async fn unknown_route_404() {
        let app = build_router(seeded_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn create_project_registers_and_appears_in_snapshot() {
        let state = seeded_state();
        let app = build_router(state);
        let body = r#"{"name":"admin","repo_path":"/admin","base_branch":"main"}"#;
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        // snapshot에 2곳(agora seed + admin)
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/snapshot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["projects"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn create_project_rejects_empty_name() {
        let app = build_router(seeded_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"","repo_path":"/x"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn snapshot_json_shape() {
        let mut conn = open_memory().unwrap();
        migrate(&mut conn).unwrap();
        let v = snapshot_json(&conn).unwrap();
        assert!(v.get("projects").is_some());
        assert!(v.get("campaigns").is_some());
        assert!(v.get("quests").is_some());
        assert!(v.get("inmail").is_some());
        assert!(v.get("taken_at").is_some());
    }
}
