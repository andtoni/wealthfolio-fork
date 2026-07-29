use axum::{body::Body, http::Request, middleware};
use tempfile::tempdir;
use tower::ServiceExt;
use wealthfolio_server::{
    api::{app_router, security_headers},
    build_state,
    config::Config,
};

#[tokio::test]
async fn healthz_works() {
    let tmp = tempdir().unwrap();
    std::env::set_var("WF_DB_PATH", tmp.path().join("test.db"));
    std::env::set_var("WF_SECRET_KEY", "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!");
    std::env::set_var("WF_LISTEN_ADDR", "127.0.0.1:0");
    let config = Config::from_env();
    let state = build_state(&config).await.unwrap();
    let app = app_router(state, &config).layer(middleware::from_fn(security_headers));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let csp = response
        .headers()
        .get("content-security-policy")
        .and_then(|value| value.to_str().ok())
        .expect("API responses must include CSP");
    assert!(csp.contains("script-src 'self' blob: https://cdn.plaid.com"));
    assert!(csp.contains("frame-src 'self' blob: about: https://cdn.plaid.com"));

    for key in ["WF_DB_PATH", "WF_SECRET_KEY", "WF_LISTEN_ADDR"] {
        std::env::remove_var(key);
    }
}
