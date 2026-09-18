use super::*;

pub(super) const LOCAL_APP_UI_SCHEME: &str = "baijimu-app";
pub(super) const LOCAL_APP_UI_BRIDGE_SCRIPT: &str = include_str!("local_app_ui_bridge.js");

/// Independent of the loopback control API: loading static UI never needs a socket or DNS.
#[derive(Clone)]
pub(super) struct LocalAppUiProtocol {
    token: String,
    diagnostics: StartupDiagnostics,
}

impl LocalAppUiProtocol {
    pub(super) fn new(diagnostics: StartupDiagnostics) -> Self {
        Self {
            token: uuid::Uuid::new_v4().simple().to_string(),
            diagnostics,
        }
    }

    pub(super) fn url(&self, app_id: &str) -> String {
        local_app_ui_url(&self.token, app_id, cfg!(windows))
    }

    pub(super) fn respond(&self, request: tauri::http::Request<Vec<u8>>) -> HttpResponse<Vec<u8>> {
        if request.method() != tauri::http::Method::GET {
            return local_app_ui_error(StatusCode::METHOD_NOT_ALLOWED, "method not allowed");
        }
        let Some((app_id, asset_path)) = local_app_ui_request(&self.token, request.uri()) else {
            self.diagnostics
                .warn("local app UI protocol request rejected: invalid_endpoint");
            return local_app_ui_error(StatusCode::NOT_FOUND, "not found");
        };
        let result = load_local_app_ui_asset(app_id, asset_path);
        match result {
            Ok((content_type, body)) => {
                self.diagnostics.info(format!(
                    "local app UI protocol request: app_id={app_id} outcome=served status=200"
                ));
                local_app_ui_response(StatusCode::OK, content_type, body)
            }
            Err(reason) => {
                self.diagnostics.warn(format!("local app UI protocol request: app_id={app_id} outcome=rejected reason={reason}"));
                local_app_ui_error(StatusCode::NOT_FOUND, "application UI resource unavailable")
            }
        }
    }
}

fn local_app_ui_url(token: &str, app_id: &str, windows: bool) -> String {
    let host = local_app_ui_host(token, app_id);
    // Wry intercepts this Windows URL before networking and restores the custom scheme.
    // Each app retains a distinct authority on every platform.
    let origin = if windows {
        format!("http://{LOCAL_APP_UI_SCHEME}.{host}")
    } else {
        format!("{LOCAL_APP_UI_SCHEME}://{host}")
    };
    format!("{origin}/{token}/{app_id}/")
}

pub(super) fn local_app_ui_host(token: &str, app_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.update([0]);
    hasher.update(app_id.as_bytes());
    format!(
        "app-{}.localhost",
        &format!("{:x}", hasher.finalize())[..20]
    )
}

fn local_app_ui_request<'a>(
    token: &str,
    uri: &'a tauri::http::Uri,
) -> Option<(&'a str, Option<&'a str>)> {
    if uri.scheme_str() != Some(LOCAL_APP_UI_SCHEME) || uri.port().is_some() {
        return None;
    }
    let mut parts = uri.path().strip_prefix('/')?.splitn(3, '/');
    if parts.next()? != token {
        return None;
    }
    let app_id = parts.next()?;
    if app_id.is_empty() || uri.authority()?.as_str() != local_app_ui_host(token, app_id) {
        return None;
    }
    let asset = parts.next()?;
    Some((app_id, if asset.is_empty() { None } else { Some(asset) }))
}

fn load_local_app_ui_asset(
    app_id: &str,
    asset: Option<&str>,
) -> Result<(&'static str, Vec<u8>), String> {
    let record = show_connector(app_id).map_err(|_| "application_not_found")?;
    read_installed_ui_asset(&record, asset)
}

fn read_installed_ui_asset(
    record: &ConnectorInstallRecord,
    asset: Option<&str>,
) -> Result<(&'static str, Vec<u8>), String> {
    let ui = record.manifest.ui.as_ref().ok_or("ui_not_declared")?;
    if asset == Some(LOCAL_APP_UI_BRIDGE_ASSET) {
        return Ok((
            "application/javascript; charset=utf-8",
            LOCAL_APP_UI_BRIDGE_SCRIPT.as_bytes().to_vec(),
        ));
    }
    let decoded = asset
        .map(urlencoding::decode)
        .transpose()
        .map_err(|_| "invalid_asset_path")?;
    let resolved =
        resolve_connector_ui_asset(Path::new(&record.package_path), ui, decoded.as_deref())
            .map_err(|_| "asset_not_found")?;
    let body = fs::read(&resolved).map_err(|_| "asset_read_failed")?;
    let body = if asset.is_none() {
        inject_local_app_ui_bridge(body)?
    } else {
        body
    };
    Ok((local_app_ui_content_type(&resolved), body))
}

pub(super) fn inject_local_app_ui_bridge(body: Vec<u8>) -> Result<Vec<u8>, String> {
    let html = String::from_utf8(body)
        .map_err(|_| "application UI entry must be UTF-8 HTML".to_string())?;
    let script = format!(r#"<script src="./{LOCAL_APP_UI_BRIDGE_ASSET}"></script>"#);
    let lower = html.to_ascii_lowercase();
    let injected = if let Some(index) = lower.find("</head>") {
        format!("{}{}{}", &html[..index], script, &html[index..])
    } else {
        format!("{script}{html}")
    };
    Ok(injected.into_bytes())
}

pub(super) fn local_app_ui_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

pub(super) fn local_app_ui_error(status: StatusCode, message: &str) -> HttpResponse<Vec<u8>> {
    local_app_ui_response(
        status,
        "text/plain; charset=utf-8",
        message.as_bytes().to_vec(),
    )
}

pub(super) fn local_app_ui_response(
    status: StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
) -> HttpResponse<Vec<u8>> {
    HttpResponse::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'none'; object-src 'none'; base-uri 'self'; form-action 'none'; frame-src 'none'; frame-ancestors tauri://localhost http://tauri.localhost http://localhost:1420",
        )
        .body(body)
        .unwrap_or_else(|_| HttpResponse::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_binds_authority_token_and_application_on_every_platform() {
        let token = "test-session-token";
        let native = local_app_ui_url(token, "first-app", false);
        let uri: tauri::http::Uri = native.parse().unwrap();
        assert_eq!(local_app_ui_request(token, &uri), Some(("first-app", None)));
        let asset: tauri::http::Uri = format!("{native}assets/main.js?v=1").parse().unwrap();
        assert_eq!(
            local_app_ui_request(token, &asset),
            Some(("first-app", Some("assets/main.js")))
        );
        let windows = local_app_ui_url(token, "first-app", true);
        assert_eq!(
            windows.replacen("http://baijimu-app.", "baijimu-app://", 1),
            native
        );
        assert_ne!(local_app_ui_url(token, "second-app", false), native);
        assert_ne!(local_app_ui_url("next-session", "first-app", false), native);
        assert!(local_app_ui_request("wrong-token", &uri).is_none());
        for invalid in [
            native.replace("/first-app/", "/second-app/"),
            native.replace("baijimu-app://", "http://"),
            native.replace(".localhost/", ".localhost:80/"),
            native.replace("baijimu-app://", "baijimu-app://other@"),
            native.replace("test-session-token", "old-session-token"),
        ] {
            assert!(local_app_ui_request(token, &invalid.parse().unwrap()).is_none());
        }
    }

    #[test]
    fn protocol_rejects_network_and_mutating_requests_before_file_access() {
        let protocol = LocalAppUiProtocol::new(StartupDiagnostics::bootstrap());
        let response = protocol.respond(
            tauri::http::Request::builder()
                .uri("http://127.0.0.1/private")
                .body(Vec::new())
                .unwrap(),
        );
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let response = protocol.respond(
            tauri::http::Request::builder()
                .method("POST")
                .uri(protocol.url("test-app"))
                .body(Vec::new())
                .unwrap(),
        );
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
    #[test]
    fn native_resources_keep_package_boundary_and_decode_asset_paths() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("ui")).unwrap();
        fs::write(
            dir.path().join("ui/index.html"),
            "<html><head></head></html>",
        )
        .unwrap();
        fs::write(dir.path().join("ui/hello world.js"), "export default 1;").unwrap();
        fs::write(dir.path().join("private.txt"), "private").unwrap();
        let record: ConnectorInstallRecord = serde_json::from_value(serde_json::json!({
            "manifest": {"schemaVersion":"3.0.0", "appId":"test-app", "name":"Test", "version":"1.0.0",
                "ui":{"type":"embedded", "entry":"ui/index.html", "defaultView":true}},
            "packagePath":dir.path(), "sourcePath":dir.path(), "reviewStatus":"LOCAL", "installedAtEpochMs":0
        })).unwrap();
        let (mime, body) = read_installed_ui_asset(&record, None).unwrap();
        assert_eq!(mime, "text/html; charset=utf-8");
        assert!(String::from_utf8(body)
            .unwrap()
            .contains(LOCAL_APP_UI_BRIDGE_ASSET));
        let (mime, body) = read_installed_ui_asset(&record, Some("hello%20world.js")).unwrap();
        assert_eq!(mime, "application/javascript; charset=utf-8");
        assert_eq!(body, b"export default 1;");
        assert!(read_installed_ui_asset(&record, Some(LOCAL_APP_UI_BRIDGE_ASSET)).is_ok());
        for path in [
            "../private.txt",
            "%2e%2e%2fprivate.txt",
            "/private.txt",
            "missing.js",
        ] {
            assert!(
                read_installed_ui_asset(&record, Some(path)).is_err(),
                "{path}"
            );
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                dir.path().join("private.txt"),
                dir.path().join("ui/escape.txt"),
            )
            .unwrap();
            assert!(read_installed_ui_asset(&record, Some("escape.txt")).is_err());
        }
    }
}
