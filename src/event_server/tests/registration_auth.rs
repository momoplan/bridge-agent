#[test]
fn service_registration_rejects_missing_token_and_compares_both_headers() {
    use axum::http::{HeaderMap, HeaderValue, StatusCode};

    let headers = HeaderMap::new();
    let missing = super::authorize_token(&None, &headers, "service registration").unwrap_err();
    assert_eq!(missing.status, StatusCode::UNAUTHORIZED);

    let blank = super::authorize_token(&Some("   ".to_string()), &headers, "service registration")
        .unwrap_err();
    assert_eq!(blank.status, StatusCode::UNAUTHORIZED);

    let mut wrong = HeaderMap::new();
    wrong.insert(
        axum::http::header::AUTHORIZATION,
        HeaderValue::from_static("Bearer other"),
    );
    wrong.insert(
        "x-bridge-agent-event-token",
        HeaderValue::from_static("also-wrong"),
    );
    assert_eq!(
        super::authorize_token(&Some("secret".to_string()), &wrong, "service registration")
            .unwrap_err()
            .status,
        StatusCode::UNAUTHORIZED
    );

    let mut bearer = HeaderMap::new();
    bearer.insert(
        axum::http::header::AUTHORIZATION,
        HeaderValue::from_static("Bearer secret"),
    );
    assert!(
        super::authorize_token(&Some("secret".to_string()), &bearer, "service registration")
            .is_ok()
    );

    let mut header = HeaderMap::new();
    header.insert(
        "x-bridge-agent-event-token",
        HeaderValue::from_static("secret"),
    );
    assert!(
        super::authorize_token(&Some("secret".to_string()), &header, "service registration")
            .is_ok()
    );
}
