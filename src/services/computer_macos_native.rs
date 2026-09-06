#[cfg(any(target_os = "macos", windows))]
fn success_outcome(data: Value) -> ServiceOutcome {
    ServiceOutcome {
        success: true,
        data: Some(data),
        error: None,
    }
}

#[cfg(target_os = "macos")]
fn with_macos_modifiers<F>(keys: &[String], action: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    let modifiers: Vec<MacKey> = keys
        .iter()
        .map(|key| parse_modifier_key(key))
        .collect::<Result<Vec<_>>>()?;
    for modifier in &modifiers {
        post_key_event(*modifier, true)?;
    }
    let result = action();
    for modifier in modifiers.into_iter().rev() {
        let _ = post_key_event(modifier, false);
    }
    result
}

#[cfg(target_os = "macos")]
fn post_key_chord(keys: &[String]) -> Result<()> {
    let mut modifiers = Vec::new();
    let mut regular_keys = Vec::new();

    for key in keys {
        let parsed = parse_key(key)?;
        if parsed.is_modifier() {
            modifiers.push(parsed);
        } else {
            regular_keys.push(parsed);
        }
    }

    for key in &modifiers {
        post_key_event(*key, true)?;
    }
    if regular_keys.is_empty() {
        for key in modifiers.iter().rev() {
            post_key_event(*key, false)?;
        }
        return Ok(());
    }
    for key in &regular_keys {
        post_key_event(*key, true)?;
        post_key_event(*key, false)?;
    }
    for key in modifiers.iter().rev() {
        post_key_event(*key, false)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn post_mouse_move(x: f64, y: f64) -> Result<()> {
    post_mouse_event(CGEventType::MouseMoved, CGMouseButton::Left, x, y)
}

#[cfg(target_os = "macos")]
fn post_mouse_click(button: CGMouseButton, x: f64, y: f64) -> Result<()> {
    let (down, up) = mouse_click_event_types(button);
    post_mouse_event(down, button, x, y)?;
    post_mouse_event(up, button, x, y)
}

#[cfg(target_os = "macos")]
fn post_drag_event(event_type: CGEventType, x: f64, y: f64) -> Result<()> {
    post_mouse_event(event_type, CGMouseButton::Left, x, y)
}

#[cfg(target_os = "macos")]
fn post_scroll(scroll_x: i64, scroll_y: i64) -> Result<()> {
    let source = event_source()?;
    let event = CGEvent::new_scroll_event(
        source,
        ScrollEventUnit::PIXEL,
        2,
        scroll_y as i32,
        scroll_x as i32,
        0,
    )
    .map_err(|_| anyhow!("failed to create scroll event"))?;
    event.post(CGEventTapLocation::HID);
    Ok(())
}

#[cfg(target_os = "macos")]
fn post_mouse_event(event_type: CGEventType, button: CGMouseButton, x: f64, y: f64) -> Result<()> {
    let source = event_source()?;
    let point = CGPoint::new(x, y);
    let event = CGEvent::new_mouse_event(source, event_type, point, button)
        .map_err(|_| anyhow!("failed to create mouse event"))?;
    event.post(CGEventTapLocation::HID);
    Ok(())
}

#[cfg(target_os = "macos")]
fn post_key_event(key: MacKey, key_down: bool) -> Result<()> {
    let source = event_source()?;
    let event = CGEvent::new_keyboard_event(source, key.code(), key_down)
        .map_err(|_| anyhow!("failed to create keyboard event"))?;
    event.post(CGEventTapLocation::HID);
    Ok(())
}

#[cfg(target_os = "macos")]
fn event_source() -> Result<CGEventSource> {
    CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| anyhow!("failed to create event source"))
}

#[cfg(target_os = "macos")]
fn mouse_click_event_types(button: CGMouseButton) -> (CGEventType, CGEventType) {
    match button {
        CGMouseButton::Left => (CGEventType::LeftMouseDown, CGEventType::LeftMouseUp),
        CGMouseButton::Right => (CGEventType::RightMouseDown, CGEventType::RightMouseUp),
        _ => (CGEventType::OtherMouseDown, CGEventType::OtherMouseUp),
    }
}

#[cfg(target_os = "macos")]
fn parse_mouse_button(value: &str) -> Result<CGMouseButton> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "left" => Ok(CGMouseButton::Left),
        "right" => Ok(CGMouseButton::Right),
        "middle" => Ok(CGMouseButton::Center),
        other => bail!("unsupported mouse button `{other}`"),
    }
}

#[cfg(target_os = "macos")]
async fn run_osascript(script: &str) -> Result<()> {
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .output()
        .await
        .context("failed to run osascript")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn apple_script_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\" & linefeed & \""),
            '\r' => {}
            _ => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}
