//! luida-server — Rust core daemon의 HTTP/SSE API (ADR-0002 브리지).
//!
//! 클라이언트(Tauri GUI / 향후 Ink TUI)가 이 API의 thin client.
//! 현재: /api/health, /api/snapshot(읽기), /api/stream(SSE). command는 후속.
//!
//! rusqlite Connection은 !Sync라 Arc<Mutex<Connection>>로 공유.

use std::convert::Infallible;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::Result;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use luida_core::{CampaignRepo, Connection, InmailRepo, ProjectRepo, QuestRepo};
use serde::Deserialize;
use serde_json::{json, Value};

/// 공유 상태 — DB connection (Mutex로 Sync 확보).
pub type AppState = Arc<Mutex<Connection>>;

/// Mutex poisoning을 복구해 잠금 (한 핸들러의 패닉이 서버 전체를 죽이지 않게). review C3.
fn lock_recover(m: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 동기 SQLite 쿼리를 blocking 풀에서 실행 (tokio 워커 블로킹 방지). review C2.
async fn snapshot_blocking(state: AppState) -> Value {
    tokio::task::spawn_blocking(move || {
        let conn = lock_recover(&state);
        snapshot_json(&conn).unwrap_or_else(|e| json!({ "error": e.to_string() }))
    })
    .await
    .unwrap_or_else(|e| json!({ "error": format!("join error: {e}") }))
}

/// 라우터 구성. 테스트는 이걸 직접 oneshot 호출.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/snapshot", get(snapshot))
        .route("/api/stream", get(stream))
        .route("/api/projects", post(create_project))
        .with_state(state)
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
        let conn = lock_recover(&state);
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
pub async fn serve(port: u16, conn: Connection) -> Result<()> {
    let state: AppState = Arc::new(Mutex::new(conn));
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
        Arc::new(Mutex::new(conn))
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
