#[cfg(target_os = "macos")]
async fn execute_macos_computer_action(
    method: &ComputerMethod,
    arguments: Value,
) -> Result<ServiceOutcome> {
    match &method.action {
        ComputerUseAction::Screenshot => capture_macos_screenshot(method).await,
        ComputerUseAction::Click => {
            let args: ComputerMouseArgs = serde_json::from_value(arguments)?;
            perform_macos_click(&args, false).await
        }
        ComputerUseAction::DoubleClick => {
            let args: ComputerMouseArgs = serde_json::from_value(arguments)?;
            perform_macos_click(&args, true).await
        }
        ComputerUseAction::Scroll => {
            let args: ComputerScrollArgs = serde_json::from_value(arguments)?;
            perform_macos_scroll(&args).await
        }
        ComputerUseAction::Type => {
            let args: ComputerTypeArgs = serde_json::from_value(arguments)?;
            perform_macos_type(&args).await
        }
        ComputerUseAction::Wait => {
            let args: ComputerWaitArgs =
                serde_json::from_value(arguments).unwrap_or(ComputerWaitArgs {
                    ms: default_wait_ms(),
                });
            sleep(Duration::from_millis(args.ms)).await;
            Ok(success_outcome(json!({ "waited_ms": args.ms })))
        }
        ComputerUseAction::Keypress => {
            let args: ComputerKeypressArgs = serde_json::from_value(arguments)?;
            perform_macos_keypress(&args).await
        }
        ComputerUseAction::Drag => {
            let args: ComputerDragArgs = serde_json::from_value(arguments)?;
            perform_macos_drag(&args).await
        }
        ComputerUseAction::Move => {
            let args: ComputerMouseArgs = serde_json::from_value(arguments)?;
            perform_macos_move(&args).await
        }
    }
}

#[cfg(target_os = "macos")]
async fn capture_macos_screenshot(method: &ComputerMethod) -> Result<ServiceOutcome> {
    let path = std::env::temp_dir().join(format!(
        "bridge-agent-screenshot-{}-{}.png",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    let mut command = Command::new("/usr/sbin/screencapture");
    command.arg("-x").arg("-t").arg("png");
    if let Some(display_id) = method.display_id {
        command.arg("-D").arg(display_id.to_string());
    }
    command.arg(&path);

    let output = command
        .output()
        .await
        .context("failed to run screencapture")?;
    if !output.status.success() {
        bail!(
            "screencapture failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let image = image::load_from_memory(&bytes).context("failed to decode screenshot")?;
    let (width, height) = image.dimensions();
    let _ = fs::remove_file(&path);

    if bytes.len() > method.upload.inline_limit_bytes {
        return upload_screenshot(method, bytes, width, height).await;
    }

    Ok(success_outcome(json!({
        "result_type": "inline_image",
        "mime_type": "image/png",
        "width": width,
        "height": height,
        "display_id": method.display_id,
        "size_bytes": bytes.len(),
        "image_base64": BASE64_STANDARD.encode(bytes),
    })))
}

#[cfg(any(target_os = "macos", windows))]
async fn upload_screenshot(
    method: &ComputerMethod,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
) -> Result<ServiceOutcome> {
    let Some(prepare_url) = method.upload_prepare_url.as_deref() else {
        return Ok(screenshot_too_large(method, bytes.len(), width, height));
    };

    let file_name = format!(
        "bridge-agent-screenshot-{}.png",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    let prepare = method
        .client
        .post(prepare_url)
        .timeout(Duration::from_secs(method.upload.timeout_secs))
        .bearer_auth(&method.relay_token)
        .json(&PrepareUploadRequest {
            agent_id: method.agent_id.clone(),
            content_type: "image/png".to_string(),
            file_name,
            size_bytes: bytes.len() as u64,
            workspace_id: method.workspace_id,
            purpose: "computer_screenshot".to_string(),
        })
        .send()
        .await
        .context("failed to request screenshot upload slot")?;

    if !prepare.status().is_success() {
        let status = prepare.status();
        let body = prepare.text().await.unwrap_or_default();
        bail!("prepare upload failed with status {}: {}", status, body);
    }

    let slot: PrepareUploadResponse = prepare
        .json()
        .await
        .context("failed to decode prepare upload response")?;
    let upload_method = slot.method.as_deref().unwrap_or("PUT").parse::<Method>()?;
    let mut upload = method
        .client
        .request(upload_method, &slot.upload_url)
        .timeout(Duration::from_secs(method.upload.timeout_secs))
        .body(bytes.clone());
    for (key, value) in &slot.headers {
        upload = upload.header(key, value);
    }
    if !slot
        .headers
        .keys()
        .any(|key| key.eq_ignore_ascii_case("content-type"))
    {
        upload = upload.header(reqwest::header::CONTENT_TYPE, "image/png");
    }

    let upload_response = upload
        .send()
        .await
        .context("failed to upload screenshot asset")?;
    if !upload_response.status().is_success() {
        let status = upload_response.status();
        let body = upload_response.text().await.unwrap_or_default();
        bail!("screenshot upload failed with status {}: {}", status, body);
    }

    Ok(success_outcome(json!({
        "result_type": "asset_ref",
        "asset_id": slot.file_id,
        "object_key": slot.object_key,
        "download_url": slot.download_url,
        "expires_at": slot.expires_at,
        "mime_type": "image/png",
        "width": width,
        "height": height,
        "display_id": method.display_id,
        "size_bytes": bytes.len(),
    })))
}

#[cfg(any(target_os = "macos", windows))]
fn screenshot_too_large(
    method: &ComputerMethod,
    size_bytes: usize,
    width: u32,
    height: u32,
) -> ServiceOutcome {
    ServiceOutcome {
        success: false,
        data: Some(json!({
            "result_type": "too_large",
            "mime_type": "image/png",
            "width": width,
            "height": height,
            "display_id": method.display_id,
            "size_bytes": size_bytes,
            "inline_limit_bytes": method.upload.inline_limit_bytes,
        })),
        error: Some(InvokeError {
            code: "PAYLOAD_TOO_LARGE".to_string(),
            message: format!(
                "screenshot is {size_bytes} bytes, exceeds inline limit {} bytes, and upload.prepare_url is not configured",
                method.upload.inline_limit_bytes
            ),
        }),
    }
}

#[cfg(target_os = "macos")]
async fn perform_macos_click(
    args: &ComputerMouseArgs,
    double_click: bool,
) -> Result<ServiceOutcome> {
    with_macos_modifiers(&args.keys, || {
        post_mouse_move(args.x, args.y)?;
        let button = parse_mouse_button(&args.button)?;
        post_mouse_click(button, args.x, args.y)?;
        if double_click {
            std::thread::sleep(Duration::from_millis(80));
            post_mouse_click(button, args.x, args.y)?;
        }
        Ok(())
    })?;

    Ok(success_outcome(json!({
        "action": if double_click { "double_click" } else { "click" },
        "x": args.x,
        "y": args.y,
        "button": args.button,
    })))
}

#[cfg(target_os = "macos")]
async fn perform_macos_move(args: &ComputerMouseArgs) -> Result<ServiceOutcome> {
    with_macos_modifiers(&args.keys, || post_mouse_move(args.x, args.y))?;
    Ok(success_outcome(json!({
        "action": "move",
        "x": args.x,
        "y": args.y,
    })))
}

#[cfg(target_os = "macos")]
async fn perform_macos_scroll(args: &ComputerScrollArgs) -> Result<ServiceOutcome> {
    with_macos_modifiers(&args.keys, || {
        post_mouse_move(args.x, args.y)?;
        post_scroll(args.scroll_x, args.scroll_y)
    })?;

    Ok(success_outcome(json!({
        "action": "scroll",
        "x": args.x,
        "y": args.y,
        "scroll_x": args.scroll_x,
        "scroll_y": args.scroll_y,
    })))
}

#[cfg(target_os = "macos")]
async fn perform_macos_type(args: &ComputerTypeArgs) -> Result<ServiceOutcome> {
    let script = format!(
        "tell application \"System Events\" to keystroke {}",
        apple_script_string(&args.text)
    );
    run_osascript(&script).await?;
    Ok(success_outcome(json!({
        "action": "type",
        "length": args.text.chars().count(),
    })))
}

#[cfg(target_os = "macos")]
async fn perform_macos_keypress(args: &ComputerKeypressArgs) -> Result<ServiceOutcome> {
    if args.keys.is_empty() {
        bail!("keypress requires at least one key");
    }
    post_key_chord(&args.keys)?;
    Ok(success_outcome(json!({
        "action": "keypress",
        "keys": args.keys,
    })))
}

#[cfg(target_os = "macos")]
async fn perform_macos_drag(args: &ComputerDragArgs) -> Result<ServiceOutcome> {
    if args.path.len() < 2 {
        bail!("drag requires at least two path points");
    }

    with_macos_modifiers(&args.keys, || {
        let start = &args.path[0];
        post_mouse_move(start.x, start.y)?;
        post_drag_event(CGEventType::LeftMouseDown, start.x, start.y)?;
        for point in args.path.iter().skip(1) {
            post_drag_event(CGEventType::LeftMouseDragged, point.x, point.y)?;
            std::thread::sleep(Duration::from_millis(16));
        }
        let end = args.path.last().expect("drag path has at least 2 points");
        post_drag_event(CGEventType::LeftMouseUp, end.x, end.y)?;
        Ok(())
    })?;

    Ok(success_outcome(json!({
        "action": "drag",
        "points": args.path.len(),
    })))
}
