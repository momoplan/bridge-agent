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
    let file_name = format!("screenshot-{}.png", Uuid::new_v4());
    let reference = method.personal_files.upload(
        &file_name, "image/png", "computer_screenshot", bytes,
    ).await?;
    Ok(success_outcome(json!({
        "result_type": "personal_file",
        "file": reference,
        "mime_type": "image/png",
        "width": width,
        "height": height,
        "display_id": method.display_id,
    })))
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
