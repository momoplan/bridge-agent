async fn upload_local_app_asset(
    State(state): State<EventServerState>,
    headers: HeaderMap,
    Json(request): Json<UploadLocalAppAssetRequest>,
) -> Result<Json<UploadLocalAppAssetResponse>, EventApiError> {
    let asset = validate_local_asset_request(&state, &headers, request)?;
    let slot = prepare_local_asset_upload(&state, &asset).await?;
    upload_local_asset_bytes(&state, &slot, &asset).await?;
    let sha256 = format!("{:x}", Sha256::digest(&asset.bytes));
    emit_audit_log(
        &state,
        "info",
        format!("connector asset {}.{} uploaded", asset.app_id, asset.purpose),
        LogMetadata::category("local_app_asset").outcome("uploaded"),
    );
    Ok(Json(UploadLocalAppAssetResponse {
        result_type: "asset_ref",
        asset_id: slot.file_id,
        object_key: slot.object_key,
        download_url: slot.download_url,
        expires_at: slot.expires_at,
        mime_type: asset.content_type,
        size_bytes: asset.size_bytes,
        sha256,
    }))
}

struct ValidatedLocalAsset {
    app_id: String,
    purpose: String,
    content_type: String,
    file_name: String,
    bytes: Vec<u8>,
    size_bytes: u64,
}

fn validate_local_asset_request(
    state: &EventServerState,
    headers: &HeaderMap,
    request: UploadLocalAppAssetRequest,
) -> Result<ValidatedLocalAsset, EventApiError> {
    let app_id = request.app_id.trim();
    let purpose = request.purpose.trim();
    let content_type = request.content_type.trim();
    if app_id.is_empty() || purpose.is_empty() || request.local_path.trim().is_empty() {
        return Err(EventApiError::new(StatusCode::BAD_REQUEST, "appId, localPath and purpose are required"));
    }
    if purpose.len() > 64 || !purpose.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')) {
        return Err(EventApiError::new(StatusCode::BAD_REQUEST, "purpose must contain at most 64 ASCII letters, digits, dot, dash or underscore"));
    }
    if !matches!(content_type, "image/png" | "image/jpeg" | "image/webp") {
        return Err(EventApiError::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "contentType must be image/png, image/jpeg or image/webp"));
    }
    let token = bearer_token(headers).ok_or_else(|| EventApiError::new(StatusCode::UNAUTHORIZED, "connector asset upload credential is required"))?;
    let data_dir = authorize_connector_asset_upload(&state.config_path, app_id, &token)
        .map_err(|err| EventApiError::new(StatusCode::FORBIDDEN, err.to_string()))?;
    let canonical_data_dir = fs::canonicalize(data_dir).map_err(internal_error)?;
    let canonical_path = fs::canonicalize(request.local_path.trim()).map_err(|err| EventApiError::new(StatusCode::BAD_REQUEST, format!("cannot read localPath: {err}")))?;
    if !canonical_path.starts_with(canonical_data_dir) || !canonical_path.is_file() {
        return Err(EventApiError::new(StatusCode::FORBIDDEN, "localPath must be a regular file inside the connector data directory"));
    }
    let bytes = fs::read(&canonical_path).map_err(internal_error)?;
    let size_bytes = bytes.len() as u64;
    if size_bytes == 0 || size_bytes > MAX_CONNECTOR_ASSET_BYTES {
        return Err(EventApiError::new(StatusCode::PAYLOAD_TOO_LARGE, format!("asset must contain 1 to {MAX_CONNECTOR_ASSET_BYTES} bytes")));
    }
    if !content_type_matches_bytes(content_type, &bytes) {
        return Err(EventApiError::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "contentType does not match the image bytes"));
    }
    Ok(ValidatedLocalAsset {
        app_id: app_id.to_string(),
        purpose: purpose.to_string(),
        content_type: content_type.to_string(),
        file_name: safe_asset_file_name(request.file_name.as_deref(), &canonical_path),
        bytes,
        size_bytes,
    })
}

fn safe_asset_file_name(requested: Option<&str>, canonical_path: &std::path::Path) -> String {
    requested
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| canonical_path.file_name().map(PathBuf::from))
        .and_then(|path| path.file_name().and_then(|value| value.to_str()).map(str::to_string))
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .unwrap_or_else(|| "connector-asset.bin".to_string())
}

async fn prepare_local_asset_upload(
    state: &EventServerState,
    asset: &ValidatedLocalAsset,
) -> Result<PrepareAssetUploadResponse, EventApiError> {
    let prepare_url = state.upload_prepare_url.as_deref().ok_or_else(|| EventApiError::new(StatusCode::SERVICE_UNAVAILABLE, "Bridge Agent upload prepare URL is not configured"))?;
    let app_scope = asset.app_id.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' }).collect::<String>();
    let response = state.http_client.post(prepare_url)
        .timeout(Duration::from_secs(state.upload_timeout_secs))
        .bearer_auth(&state.relay_token)
        .json(&PrepareAssetUploadRequest {
            agent_id: state.agent_id.clone(),
            workspace_id: state.workspace_id,
            purpose: format!("connector_{app_scope}_{}", asset.purpose),
            content_type: asset.content_type.clone(),
            file_name: asset.file_name.clone(),
            size_bytes: asset.size_bytes,
        })
        .send().await.map_err(|err| EventApiError::new(StatusCode::BAD_GATEWAY, err.to_string()))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(EventApiError::new(StatusCode::BAD_GATEWAY, format!("prepare upload returned {status}: {}", body.chars().take(240).collect::<String>())));
    }
    response.json().await.map_err(internal_error)
}

async fn upload_local_asset_bytes(
    state: &EventServerState,
    slot: &PrepareAssetUploadResponse,
    asset: &ValidatedLocalAsset,
) -> Result<(), EventApiError> {
    let method = slot.method.as_deref().unwrap_or("PUT").parse::<Method>().map_err(internal_error)?;
    let mut upload = state.http_client.request(method, &slot.upload_url)
        .timeout(Duration::from_secs(state.upload_timeout_secs)).body(asset.bytes.clone());
    for (name, value) in &slot.headers {
        upload = upload.header(name, value);
    }
    if !slot.headers.keys().any(|name| name.eq_ignore_ascii_case("content-type")) {
        upload = upload.header(reqwest::header::CONTENT_TYPE, &asset.content_type);
    }
    let response = upload.send().await.map_err(|err| EventApiError::new(StatusCode::BAD_GATEWAY, err.to_string()))?;
    if !response.status().is_success() {
        return Err(EventApiError::new(StatusCode::BAD_GATEWAY, format!("asset upload returned {}", response.status())));
    }
    Ok(())
}

fn content_type_matches_bytes(content_type: &str, bytes: &[u8]) -> bool {
    match content_type {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/webp" => bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        _ => false,
    }
}
