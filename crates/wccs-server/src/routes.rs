//! HTTP 入口按调用方组织。各业务处理函数负责核对操作权限与设备绑定。
use crate::{
    ApiError, App, auth, contest_binding, contest_control, control, deployment, device,
    diagnostics, distribution, domjudge, events, health, image, lifecycle, maintenance, management,
    operators, preparation, presentation, recovery, roster, seating, waiting,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request},
    http::HeaderValue,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
use std::time::Duration;
use tower_http::services::ServeDir;

pub fn router(app: App) -> Router {
    management_routes()
        .merge(device_routes(&app))
        .fallback_service(ServeDir::new(&app.config.static_dir))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .layer(middleware::from_fn(response_policy))
        .with_state(app)
}

async fn health_status() -> Json<serde_json::Value> {
    Json(json!({
        "service": "wccs",
        "protocol_version": wccs_protocol::PROTOCOL_VERSION,
        "server_time": wccs_protocol::now(),
    }))
}

fn management_routes() -> Router<App> {
    Router::new()
        .route("/api/health", get(health_status))
        .route("/api/ready", get(deployment::ready))
        .route("/api/waiting/preview", post(waiting::preview))
        .route("/api/waiting/commit", post(waiting::commit))
        .route("/api/waiting", get(waiting::list))
        .route(
            "/api/presentation",
            get(presentation::get).post(presentation::save),
        )
        .route("/api/maintenance/preview", post(maintenance::preview))
        .route("/api/maintenance/commit", post(maintenance::commit))
        .route(
            "/api/preparation",
            get(preparation::get).post(preparation::save),
        )
        .route("/api/contest-control", get(contest_control::list))
        .route(
            "/api/contest-control/preview",
            post(contest_control::preview),
        )
        .route("/api/contest-control/commit", post(contest_control::commit))
        .route("/api/contest-control/cancel", post(contest_control::cancel))
        .route("/api/domjudge", get(domjudge::list))
        .route("/api/domjudge/bind", post(contest_binding::bind))
        .route("/api/domjudge/choices", get(contest_binding::choices))
        .route("/api/domjudge/preview", post(domjudge::preview))
        .route("/api/domjudge/commit", post(domjudge::commit))
        .route("/api/distribution", get(distribution::list))
        .route("/api/distribution/artifacts", post(distribution::create))
        .route(
            "/api/distribution/artifacts/{id}/{index}",
            post(distribution::upload),
        )
        .route("/api/distribution/preview", post(distribution::preview))
        .route("/api/distribution/commit", post(distribution::commit))
        .route("/api/distribution/cancel", post(distribution::cancel))
        .route("/api/maintenance", get(maintenance::list))
        .route("/api/maintenance/cancel", post(maintenance::cancel))
        .route("/api/login", post(auth::login))
        .route("/api/session", get(auth::session))
        .route("/api/logout", post(auth::logout))
        .route("/api/operators", get(operators::list).post(operators::save))
        .route("/api/recovery", get(recovery::list))
        .route("/api/recovery/manifest/{job}", get(recovery::manifest))
        .route("/api/recovery/archives/{job}", get(recovery::download))
        .route("/api/events", get(events::list))
        .route("/api/events/action", post(events::action))
        .route("/api/events/{id}", get(events::detail))
        .route("/api/state", get(management::state))
        .route("/api/roster/preview", post(roster::preview))
        .route("/api/roster/commit", post(roster::commit))
        .route("/api/roster/{id}/history", get(roster::history))
        .route("/api/devices", post(management::create_device))
        .route("/api/registration-window", post(lifecycle::window))
        .route("/api/devices/action", post(lifecycle::action))
        .route("/api/devices/{id}/history", get(lifecycle::history))
        .route("/api/configuration", post(management::publish))
        .route("/api/recheck", post(management::recheck))
        .route("/api/rooms/preview", post(seating::preview))
        .route("/api/rooms/save", post(seating::save))
        .route("/api/seating/check-preview", post(seating::check_preview))
        .route("/api/seating/check", post(seating::check))
        .route("/api/seating/review", post(seating::review))
        .route("/api/images/action", post(image::action))
}

fn device_routes(app: &App) -> Router<App> {
    Router::new()
        .route("/api/device/control", get(control::upgrade))
        .route("/api/device/diagnostics/context", get(diagnostics::context))
        .route("/api/device/diagnostics/probe", get(diagnostics::probe))
        .route("/api/device/presentation", post(presentation::device))
        .route(
            "/api/device/presentation/report",
            post(presentation::report),
        )
        .route("/api/device/health", post(health::report))
        .route("/api/device/diagnostics", post(diagnostics::receive))
        .route("/api/device/distribution/poll", post(distribution::poll))
        .route(
            "/api/device/distribution/progress",
            post(distribution::progress),
        )
        .route(
            "/api/device/desktop-files/{job}/{artifact}/{index}",
            get(distribution::desktop_chunk),
        )
        .route(
            "/api/device/distribution/{job}/{index}",
            get(distribution::download),
        )
        .route("/api/device/maintenance/poll", post(maintenance::poll))
        .route(
            "/api/device/maintenance/authorize",
            post(maintenance::authorize),
        )
        // 外层先鉴权并取得配额，再读取放宽大小限制的大回执。
        .route(
            "/api/device/maintenance/result",
            post(maintenance::result)
                .layer(DefaultBodyLimit::max(
                    wccs_protocol::recovery::MAX_MANIFEST_BYTES,
                ))
                .layer(axum::middleware::from_fn_with_state(
                    app.clone(),
                    authenticate_receipt,
                )),
        )
        .route(
            "/api/device/recovery/{job}/{file}/{index}",
            post(recovery::upload),
        )
        .route("/api/device/recovery/{job}/finish", post(recovery::finish))
        .route("/api/device/rotate", post(lifecycle::rotate))
        .route("/api/device/register", post(device::register))
        .route("/api/device/image-register", post(image::register))
        .route("/api/device/heartbeat", post(device::heartbeat))
        .route("/api/device/result", post(device::result))
}
async fn response_policy(request: Request, next: Next) -> Response {
    let mut response = match tokio::time::timeout(Duration::from_secs(15), next.run(request)).await
    {
        Ok(response) => response,
        Err(_) => ApiError::new(504, "请求超时，可使用相同请求编号重试").into_response(),
    };
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "same-origin"),
        (
            "content-security-policy",
            "default-src 'self'; style-src 'self'; font-src 'self'; script-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
        ),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}

async fn authenticate_receipt(
    axum::extract::State(app): axum::extract::State<App>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    {
        let mut db = app.db.lock().await?;
        let tx = db.transaction().await?;
        auth::device(&tx, request.headers()).await?;
        tx.commit().await?;
    }
    let _permit = app
        .receipt_slots
        .try_acquire()
        .map_err(|_| ApiError::new(429, "维护回执接收繁忙，请稍后重试"))?;
    Ok(next.run(request).await)
}
