use std::{any::Any, panic::catch_unwind};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use tempfile::tempdir;
use wealthfolio_server::config::Config;

const SECRET_VARS: [&str; 6] = [
    "WF_SECRET_KEY",
    "WF_SECRET_KEY_FILE",
    "WF_AUTH_PASSWORD_HASH",
    "WF_AUTH_PASSWORD_HASH_FILE",
    "WF_OIDC_CLIENT_SECRET",
    "WF_OIDC_CLIENT_SECRET_FILE",
];

fn clear_secret_vars() {
    for name in SECRET_VARS {
        std::env::remove_var(name);
    }
}

fn panic_message(payload: Box<dyn Any + Send>) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(ToString::to_string))
        .unwrap_or_default()
}

#[test]
fn secret_file_sources_allow_final_newlines_and_fail_closed() {
    let tmp = tempdir().unwrap();
    let key_file = tmp.path().join("key");
    let password_file = tmp.path().join("password");
    let oidc_secret_file = tmp.path().join("oidc-secret");
    let key = BASE64.encode([9u8; 32]);
    std::fs::write(&key_file, format!("{key}\n")).unwrap();
    std::fs::write(&password_file, "$argon2id$test\n").unwrap();
    std::fs::write(&oidc_secret_file, "oidc-test-secret\r\n").unwrap();

    clear_secret_vars();
    std::env::set_var("WF_LISTEN_ADDR", "127.0.0.1:8088");
    std::env::set_var("WF_CORS_ALLOW_ORIGINS", "http://localhost:3000");
    std::env::set_var("WF_OIDC_ISSUER_URL", "https://issuer.example");
    std::env::set_var("WF_OIDC_CLIENT_ID", "test-client");
    std::env::set_var("WF_OIDC_REDIRECT_URL", "https://app.example/callback");
    std::env::set_var("WF_OIDC_ALLOW_ANY", "true");
    std::env::set_var("WF_SECRET_KEY_FILE", &key_file);
    std::env::set_var("WF_AUTH_PASSWORD_HASH_FILE", &password_file);
    std::env::set_var("WF_OIDC_CLIENT_SECRET_FILE", &oidc_secret_file);

    let config = Config::from_env();
    assert_eq!(config.raw_secret_key, [9u8; 32]);
    assert_eq!(
        config.auth.unwrap().password_hash.as_deref(),
        Some("$argon2id$test")
    );
    assert_eq!(
        config.oidc.unwrap().client_secret.as_deref(),
        Some("oidc-test-secret")
    );

    std::env::set_var("WF_SECRET_KEY", "direct-secret-must-not-leak");
    let dual_source = match catch_unwind(Config::from_env) {
        Ok(_) => panic!("dual source must fail"),
        Err(payload) => panic_message(payload),
    };
    assert!(dual_source.contains("WF_SECRET_KEY"));
    assert!(!dual_source.contains("direct-secret-must-not-leak"));

    std::env::remove_var("WF_SECRET_KEY");
    std::fs::write(&key_file, "").unwrap();
    let empty_file = match catch_unwind(Config::from_env) {
        Ok(_) => panic!("empty file must fail"),
        Err(payload) => panic_message(payload),
    };
    assert!(empty_file.contains("WF_SECRET_KEY_FILE"));

    std::env::set_var("WF_SECRET_KEY_FILE", tmp.path().join("missing"));
    let unreadable_file = match catch_unwind(Config::from_env) {
        Ok(_) => panic!("unreadable file must fail"),
        Err(payload) => panic_message(payload),
    };
    assert!(unreadable_file.contains("WF_SECRET_KEY_FILE"));

    clear_secret_vars();
    for name in [
        "WF_LISTEN_ADDR",
        "WF_CORS_ALLOW_ORIGINS",
        "WF_OIDC_ISSUER_URL",
        "WF_OIDC_CLIENT_ID",
        "WF_OIDC_REDIRECT_URL",
        "WF_OIDC_ALLOW_ANY",
    ] {
        std::env::remove_var(name);
    }
}
