#[cfg(windows)]
async fn execute_windows_computer_action(
    method: &ComputerMethod,
    arguments: Value,
) -> Result<ServiceOutcome> {
    match &method.action {
        ComputerUseAction::Screenshot => capture_windows_screenshot(method).await,
        ComputerUseAction::Click => {
            let args: ComputerMouseArgs = serde_json::from_value(arguments)?;
            perform_windows_click(&args, false).await
        }
        ComputerUseAction::DoubleClick => {
            let args: ComputerMouseArgs = serde_json::from_value(arguments)?;
            perform_windows_click(&args, true).await
        }
        ComputerUseAction::Scroll => {
            let args: ComputerScrollArgs = serde_json::from_value(arguments)?;
            perform_windows_scroll(&args).await
        }
        ComputerUseAction::Type => {
            let args: ComputerTypeArgs = serde_json::from_value(arguments)?;
            perform_windows_type(&args).await
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
            perform_windows_keypress(&args).await
        }
        ComputerUseAction::Drag => {
            let args: ComputerDragArgs = serde_json::from_value(arguments)?;
            perform_windows_drag(&args).await
        }
        ComputerUseAction::Move => {
            let args: ComputerMouseArgs = serde_json::from_value(arguments)?;
            perform_windows_move(&args).await
        }
    }
}

#[cfg(windows)]
async fn capture_windows_screenshot(method: &ComputerMethod) -> Result<ServiceOutcome> {
    let capture = capture_windows_monitor_png(method.display_id)?;
    if capture.bytes.len() > method.upload.inline_limit_bytes {
        return upload_screenshot(method, capture.bytes, capture.width, capture.height).await;
    }

    Ok(success_outcome(json!({
        "result_type": "inline_image",
        "mime_type": "image/png",
        "width": capture.width,
        "height": capture.height,
        "display_id": capture.display_id,
        "size_bytes": capture.bytes.len(),
        "image_base64": BASE64_STANDARD.encode(capture.bytes),
    })))
}

#[cfg(windows)]
async fn perform_windows_click(
    args: &ComputerMouseArgs,
    double_click: bool,
) -> Result<ServiceOutcome> {
    with_windows_modifiers(&args.keys, || {
        set_windows_cursor_position(args.x, args.y)?;
        let (down, up) = windows_mouse_button_flags(&args.button)?;
        send_windows_mouse_input(0, 0, down, 0)?;
        send_windows_mouse_input(0, 0, up, 0)?;
        if double_click {
            std::thread::sleep(Duration::from_millis(80));
            send_windows_mouse_input(0, 0, down, 0)?;
            send_windows_mouse_input(0, 0, up, 0)?;
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

#[cfg(windows)]
async fn perform_windows_move(args: &ComputerMouseArgs) -> Result<ServiceOutcome> {
    with_windows_modifiers(&args.keys, || set_windows_cursor_position(args.x, args.y))?;
    Ok(success_outcome(json!({
        "action": "move",
        "x": args.x,
        "y": args.y,
    })))
}

#[cfg(windows)]
async fn perform_windows_scroll(args: &ComputerScrollArgs) -> Result<ServiceOutcome> {
    with_windows_modifiers(&args.keys, || {
        set_windows_cursor_position(args.x, args.y)?;
        if args.scroll_y != 0 {
            let delta = scale_windows_wheel_delta(args.scroll_y)?;
            send_windows_mouse_input(0, 0, MOUSEEVENTF_WHEEL, delta)?;
        }
        if args.scroll_x != 0 {
            let delta = scale_windows_wheel_delta(args.scroll_x)?;
            send_windows_mouse_input(0, 0, MOUSEEVENTF_HWHEEL, delta)?;
        }
        Ok(())
    })?;

    Ok(success_outcome(json!({
        "action": "scroll",
        "x": args.x,
        "y": args.y,
        "scroll_x": args.scroll_x,
        "scroll_y": args.scroll_y,
    })))
}

#[cfg(windows)]
async fn perform_windows_type(args: &ComputerTypeArgs) -> Result<ServiceOutcome> {
    send_windows_unicode_text(&args.text)?;
    Ok(success_outcome(json!({
        "action": "type",
        "length": args.text.chars().count(),
    })))
}

#[cfg(windows)]
async fn perform_windows_keypress(args: &ComputerKeypressArgs) -> Result<ServiceOutcome> {
    if args.keys.is_empty() {
        bail!("keypress requires at least one key");
    }
    send_windows_key_chord(&args.keys)?;
    Ok(success_outcome(json!({
        "action": "keypress",
        "keys": args.keys,
    })))
}

#[cfg(windows)]
async fn perform_windows_drag(args: &ComputerDragArgs) -> Result<ServiceOutcome> {
    if args.path.len() < 2 {
        bail!("drag requires at least two path points");
    }

    with_windows_modifiers(&args.keys, || {
        let start = &args.path[0];
        set_windows_cursor_position(start.x, start.y)?;
        send_windows_mouse_input(0, 0, MOUSEEVENTF_LEFTDOWN, 0)?;
        for point in args.path.iter().skip(1) {
            set_windows_cursor_position(point.x, point.y)?;
            std::thread::sleep(Duration::from_millis(16));
        }
        send_windows_mouse_input(0, 0, MOUSEEVENTF_LEFTUP, 0)?;
        Ok(())
    })?;

    Ok(success_outcome(json!({
        "action": "drag",
        "points": args.path.len(),
    })))
}
