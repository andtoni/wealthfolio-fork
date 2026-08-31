use wealthfolio_connect::DEFAULT_CLOUD_API_URL;

pub fn connect_sync_enabled() -> bool {
    cfg!(feature = "connect-sync")
}

pub fn device_sync_enabled() -> bool {
    cfg!(feature = "device-sync")
}

pub fn investec_card_lifecycle_link_enabled() -> bool {
    std::env::var("WF_INVESTEC_CARD_LIFECYCLE_LINK_ENABLED")
        .ok()
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

pub fn cloud_sync_enabled() -> bool {
    connect_sync_enabled() || device_sync_enabled()
}

pub fn cloud_api_base_url() -> Option<String> {
    if !cloud_sync_enabled() {
        return None;
    }

    std::env::var("CONNECT_API_URL")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| Some(DEFAULT_CLOUD_API_URL.to_string()))
}
