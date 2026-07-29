use std::{sync::Arc, time::Duration};

use axum::{
    body::{to_bytes, Body},
    extract::Extension,
    http::{header, Request, StatusCode},
    routing::get,
    Router,
};
use tower::ServiceExt;
use wealthfolio_server::auth::{
    authenticated_router, AuthConfig, AuthManager, AuthenticatedSubject, CookieSecurePolicy,
};

fn manager() -> Arc<AuthManager> {
    Arc::new(
        AuthManager::new(&AuthConfig {
            password_hash: None,
            password_subject: "urn:wealthfolio:password#bound-owner".to_owned(),
            jwt_secret: vec![7; 32],
            access_token_ttl: Duration::from_secs(3600),
            cookie_secure: CookieSecurePolicy::Never,
            bridge_service_token: None,
            bridge_service_subject: None,
        })
        .unwrap(),
    )
}

async fn subject(Extension(subject): Extension<AuthenticatedSubject>) -> String {
    subject.as_str().to_owned()
}

#[tokio::test]
async fn signed_claim_subject_is_inserted_into_request_extensions() {
    let auth = manager();
    let token = auth.issue_password_token().unwrap();
    let app = authenticated_router(Router::new().route("/household", get(subject)), auth);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/household")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        "urn:wealthfolio:password#bound-owner"
    );
}

#[tokio::test]
async fn client_supplied_subject_header_cannot_substitute_signed_subject() {
    let auth = manager();
    let token = auth.issue_password_token().unwrap();
    let app = authenticated_router(Router::new().route("/household", get(subject)), auth);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/household")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header("x-authenticated-subject", "https://id.example.test#jordan")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        "urn:wealthfolio:password#bound-owner"
    );
}

#[tokio::test]
async fn missing_or_legacy_constant_subject_is_rejected() {
    let auth = manager();
    let app = authenticated_router(
        Router::new().route("/household", get(subject)),
        auth.clone(),
    );

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/household")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
}
