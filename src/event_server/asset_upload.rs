async fn upload_local_app_asset(
    State(state): State<EventServerState>,
    headers: HeaderMap,
    Json(request): Json<UploadLocalAppAssetRequest>,
) -> Result<Json<UploadLocalAppAssetResponse>, asset_protocol::AssetUploadError> {
    let asset = validate_local_asset_request(&state, &headers, request)?;
    let slot = prepare_local_asset_upload(&state, &asset).await?;
    upload_local_asset_bytes(&state, &slot, &asset).await?;
    let completed = asset_protocol::complete(&state, &slot, &asset).await?;
    let sha256 = format!("{:x}", Sha256::digest(&asset.bytes));
    emit_audit_log(
        &state,
        "info",
        format!("connector asset {}.{} uploaded", asset.app_id, asset.purpose),
        LogMetadata::category("local_app_asset").outcome("uploaded"),
    );
    Ok(Json(UploadLocalAppAssetResponse {
        result_type: "asset_ref",
        asset_id: completed.file_id,
        object_key: Some(completed.path),
        download_url: Some(completed.download_url),
        expires_at: Some(completed.expires_at),
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
) -> Result<PrepareAssetUploadResponse, asset_protocol::AssetUploadError> {
    let slot: PrepareAssetUploadResponse = asset_protocol::post(state, "prepare", &PrepareAssetUploadRequest {
        contract_version: asset_protocol::ContractVersion::Current,
        agent_id: state.agent_id.clone(), workspace_id: state.workspace_id,
        app_id: asset.app_id.clone(), purpose: asset.purpose.clone(),
        content_type: asset.content_type.clone(), file_name: asset.file_name.clone(), size_bytes: asset.size_bytes,
    }).await?;
    let _version = &slot.contract_version;
    if uuid::Uuid::parse_str(&slot.file_id).is_err() || slot.object_key.is_empty() || slot.method != "PUT" {
        return Err(asset_protocol::AssetUploadError::new(StatusCode::BAD_GATEWAY, "UPLOAD_RESPONSE_INVALID", "文件服务返回的上传票据无效", false, "prepare"));
    }
    Ok(slot)
}

async fn upload_local_asset_bytes(
    state: &EventServerState,
    slot: &PrepareAssetUploadResponse,
    asset: &ValidatedLocalAsset,
) -> Result<(), asset_protocol::AssetUploadError> {
    crate::file_transfer::put_source(&state.http_client, &slot.upload_url, &slot.headers,
        crate::file_transfer::UploadSource::Bytes(asset.bytes.clone()), asset.size_bytes,
        Arc::new(std::sync::atomic::AtomicU64::new(0))).await
        .map_err(|error| asset_protocol::AssetUploadError::new(StatusCode::BAD_GATEWAY, &error.code, error.message, false, "transfer"))?;
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
