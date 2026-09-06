#[cfg(windows)]
#[derive(Clone, Copy)]
struct WindowsKey {
    virtual_key: u16,
}

#[cfg(windows)]
impl WindowsKey {
    fn is_modifier(self) -> bool {
        matches!(self.virtual_key, VK_SHIFT | VK_CONTROL | VK_MENU | VK_LWIN)
    }
}

#[cfg(windows)]
struct WindowsMonitorCapture {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    display_id: Option<u32>,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct WindowsMonitorBounds {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

#[cfg(windows)]
fn with_windows_modifiers<F>(keys: &[String], action: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    let modifiers: Vec<WindowsKey> = keys
        .iter()
        .map(|key| parse_windows_modifier_key(key))
        .collect::<Result<Vec<_>>>()?;
    for key in &modifiers {
        send_windows_virtual_key(key.virtual_key, true)?;
    }
    let result = action();
    for key in modifiers.into_iter().rev() {
        let _ = send_windows_virtual_key(key.virtual_key, false);
    }
    result
}

#[cfg(windows)]
fn send_windows_key_chord(keys: &[String]) -> Result<()> {
    let mut modifiers = Vec::new();
    let mut regular_keys = Vec::new();

    for key in keys {
        let parsed = parse_windows_key(key)?;
        if parsed.is_modifier() {
            modifiers.push(parsed);
        } else {
            regular_keys.push(parsed);
        }
    }

    for key in &modifiers {
        send_windows_virtual_key(key.virtual_key, true)?;
    }
    if regular_keys.is_empty() {
        for key in modifiers.iter().rev() {
            send_windows_virtual_key(key.virtual_key, false)?;
        }
        return Ok(());
    }
    for key in &regular_keys {
        send_windows_virtual_key(key.virtual_key, true)?;
        send_windows_virtual_key(key.virtual_key, false)?;
    }
    for key in modifiers.iter().rev() {
        send_windows_virtual_key(key.virtual_key, false)?;
    }
    Ok(())
}

#[cfg(windows)]
fn send_windows_unicode_text(text: &str) -> Result<()> {
    for unit in text.encode_utf16() {
        let down = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: 0,
                    wScan: unit,
                    dwFlags: KEYEVENTF_UNICODE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let up = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: 0,
                    wScan: unit,
                    dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        send_windows_inputs(&[down, up])?;
    }
    Ok(())
}

#[cfg(windows)]
fn send_windows_virtual_key(virtual_key: u16, key_down: bool) -> Result<()> {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: 0,
                dwFlags: if key_down { 0 } else { KEYEVENTF_KEYUP },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send_windows_inputs(&[input])
}

#[cfg(windows)]
fn send_windows_mouse_input(dx: i32, dy: i32, flags: u32, mouse_data: u32) -> Result<()> {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send_windows_inputs(&[input])
}

#[cfg(windows)]
fn send_windows_inputs(inputs: &[INPUT]) -> Result<()> {
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        let error = unsafe { GetLastError() };
        bail!("SendInput failed with error {}", error);
    }
    Ok(())
}

#[cfg(windows)]
fn set_windows_cursor_position(x: f64, y: f64) -> Result<()> {
    let x = round_f64_to_i32(x, "x")?;
    let y = round_f64_to_i32(y, "y")?;
    if unsafe { SetCursorPos(x, y) } == 0 {
        let error = unsafe { GetLastError() };
        bail!("SetCursorPos failed with error {}", error);
    }
    Ok(())
}

#[cfg(windows)]
fn windows_mouse_button_flags(button: &str) -> Result<(u32, u32)> {
    match button.trim().to_ascii_lowercase().as_str() {
        "" | "left" => Ok((MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)),
        "right" => Ok((MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)),
        "middle" => Ok((MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP)),
        other => bail!("unsupported mouse button `{other}`"),
    }
}

#[cfg(windows)]
fn scale_windows_wheel_delta(amount: i64) -> Result<u32> {
    let delta = amount
        .checked_mul(WHEEL_DELTA as i64)
        .ok_or_else(|| anyhow!("scroll delta is too large"))?;
    let delta = i32::try_from(delta).map_err(|_| anyhow!("scroll delta is too large"))?;
    Ok(delta as u32)
}

#[cfg(windows)]
fn parse_windows_modifier_key(value: &str) -> Result<WindowsKey> {
    match parse_windows_key(value)? {
        key if key.is_modifier() => Ok(key),
        _ => bail!("mouse action modifiers only support Shift / Ctrl / Alt / Win"),
    }
}

#[cfg(windows)]
fn parse_windows_key(value: &str) -> Result<WindowsKey> {
    let virtual_key = match value.trim().to_ascii_uppercase().as_str() {
        "SHIFT" => VK_SHIFT,
        "CTRL" | "CONTROL" => VK_CONTROL,
        "ALT" | "OPTION" => VK_MENU,
        "META" | "COMMAND" | "CMD" | "WIN" | "WINDOWS" => VK_LWIN,
        "ENTER" | "RETURN" => VK_RETURN,
        "TAB" => VK_TAB,
        "SPACE" => VK_SPACE,
        "ESC" | "ESCAPE" => VK_ESCAPE,
        "UP" | "ARROWUP" => VK_UP,
        "DOWN" | "ARROWDOWN" => VK_DOWN,
        "LEFT" | "ARROWLEFT" => VK_LEFT,
        "RIGHT" | "ARROWRIGHT" => VK_RIGHT,
        "HOME" => VK_HOME,
        "END" => VK_END,
        "PAGEUP" => VK_PRIOR,
        "PAGEDOWN" => VK_NEXT,
        "A" => 0x41,
        "B" => 0x42,
        "C" => 0x43,
        "D" => 0x44,
        "E" => 0x45,
        "F" => 0x46,
        "G" => 0x47,
        "H" => 0x48,
        "I" => 0x49,
        "J" => 0x4A,
        "K" => 0x4B,
        "L" => 0x4C,
        "M" => 0x4D,
        "N" => 0x4E,
        "O" => 0x4F,
        "P" => 0x50,
        "Q" => 0x51,
        "R" => 0x52,
        "S" => 0x53,
        "T" => 0x54,
        "U" => 0x55,
        "V" => 0x56,
        "W" => 0x57,
        "X" => 0x58,
        "Y" => 0x59,
        "Z" => 0x5A,
        "0" => 0x30,
        "1" => 0x31,
        "2" => 0x32,
        "3" => 0x33,
        "4" => 0x34,
        "5" => 0x35,
        "6" => 0x36,
        "7" => 0x37,
        "8" => 0x38,
        "9" => 0x39,
        other => bail!("unsupported key `{other}`"),
    };
    Ok(WindowsKey { virtual_key })
}

#[cfg(windows)]
fn round_f64_to_i32(value: f64, label: &str) -> Result<i32> {
    if !value.is_finite() {
        bail!("{label} must be finite");
    }
    let rounded = value.round();
    if rounded < i32::MIN as f64 || rounded > i32::MAX as f64 {
        bail!("{label} is out of range");
    }
    Ok(rounded as i32)
}

#[cfg(any(windows, test))]
fn bgra_to_rgba(bgra: &[u8]) -> Result<Vec<u8>> {
    let (pixels, remainder) = bgra.as_chunks::<4>();
    if !remainder.is_empty() {
        bail!("BGRA image buffer length must be divisible by four");
    }

    let mut rgba = Vec::with_capacity(bgra.len());
    for &[blue, green, red, _alpha] in pixels {
        rgba.extend_from_slice(&[red, green, blue, 255]);
    }
    Ok(rgba)
}

#[cfg(windows)]
fn capture_windows_monitor_png(display_id: Option<u32>) -> Result<WindowsMonitorCapture> {
    let bounds = windows_monitor_bounds(display_id)?;
    let width_u32 = u32::try_from(bounds.width).map_err(|_| anyhow!("invalid monitor width"))?;
    let height_u32 = u32::try_from(bounds.height).map_err(|_| anyhow!("invalid monitor height"))?;

    let screen_dc = unsafe { GetDC(std::ptr::null_mut()) };
    if screen_dc.is_null() {
        bail!("GetDC failed");
    }
    let screen_dc_guard = ReleaseDcGuard { hdc: screen_dc };

    let memory_dc = unsafe { CreateCompatibleDC(screen_dc) };
    if memory_dc.is_null() {
        bail!("CreateCompatibleDC failed");
    }
    let memory_dc_guard = DeleteDcGuard { hdc: memory_dc };

    let bitmap = unsafe { CreateCompatibleBitmap(screen_dc, bounds.width, bounds.height) };
    if bitmap.is_null() {
        bail!("CreateCompatibleBitmap failed");
    }
    let bitmap_guard = DeleteObjectGuard {
        handle: bitmap as HGDIOBJ,
    };

    let previous = unsafe { SelectObject(memory_dc, bitmap as HGDIOBJ) };
    if previous.is_null() {
        bail!("SelectObject failed");
    }
    let selection_guard = SelectObjectGuard {
        hdc: memory_dc,
        previous,
    };

    if unsafe {
        BitBlt(
            memory_dc,
            0,
            0,
            bounds.width,
            bounds.height,
            screen_dc,
            bounds.left,
            bounds.top,
            SRCCOPY | CAPTUREBLT,
        )
    } == 0
    {
        let error = unsafe { GetLastError() };
        bail!("BitBlt failed with error {}", error);
    }

    let mut bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: bounds.width,
            biHeight: -bounds.height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: (width_u32 * height_u32 * 4),
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        ..Default::default()
    };
    let mut bgra = vec![0u8; (width_u32 as usize) * (height_u32 as usize) * 4];
    let rows = unsafe {
        GetDIBits(
            memory_dc,
            bitmap as HBITMAP,
            0,
            height_u32,
            bgra.as_mut_ptr().cast(),
            &mut bitmap_info,
            DIB_RGB_COLORS,
        )
    };
    if rows == 0 {
        let error = unsafe { GetLastError() };
        bail!("GetDIBits failed with error {}", error);
    }

    let _ = selection_guard;
    let _ = bitmap_guard;
    let _ = memory_dc_guard;
    let _ = screen_dc_guard;

    let bytes = encode_windows_screenshot(width_u32, height_u32, &bgra)?;

    Ok(WindowsMonitorCapture {
        bytes,
        width: width_u32,
        height: height_u32,
        display_id,
    })
}

#[cfg(windows)]
fn encode_windows_screenshot(width: u32, height: u32, bgra: &[u8]) -> Result<Vec<u8>> {
    let image = image::RgbaImage::from_raw(width, height, bgra_to_rgba(bgra)?)
        .ok_or_else(|| anyhow!("failed to build RGBA image"))?;
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .context("failed to encode screenshot")?;
    Ok(bytes)
}

#[cfg(windows)]
fn windows_monitor_bounds(display_id: Option<u32>) -> Result<WindowsMonitorBounds> {
    if let Some(display_id) = display_id {
        let monitors = list_windows_monitors()?;
        let bounds = monitors
            .get(display_id as usize)
            .copied()
            .ok_or_else(|| anyhow!("display_id {} does not exist", display_id))?;
        return Ok(bounds);
    }

    let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    if width <= 0 || height <= 0 {
        bail!("virtual screen metrics are invalid");
    }
    Ok(WindowsMonitorBounds {
        left,
        top,
        width,
        height,
    })
}

#[cfg(windows)]
fn list_windows_monitors() -> Result<Vec<WindowsMonitorBounds>> {
    let mut monitors = Vec::new();
    let ok = unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect_windows_monitor),
            (&mut monitors as *mut Vec<WindowsMonitorBounds>) as LPARAM,
        )
    };
    if ok == 0 {
        let error = unsafe { GetLastError() };
        bail!("EnumDisplayMonitors failed with error {}", error);
    }
    if monitors.is_empty() {
        bail!("no monitors found");
    }
    Ok(monitors)
}

#[cfg(windows)]
unsafe extern "system" fn collect_windows_monitor(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> i32 {
    let monitors = &mut *(data as *mut Vec<WindowsMonitorBounds>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(monitor, &mut info as *mut _ as *mut _) == 0 {
        return 1;
    }
    let rect = info.monitorInfo.rcMonitor;
    monitors.push(WindowsMonitorBounds {
        left: rect.left,
        top: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    });
    1
}

#[cfg(windows)]
struct ReleaseDcGuard {
    hdc: HDC,
}

#[cfg(windows)]
impl Drop for ReleaseDcGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseDC(std::ptr::null_mut(), self.hdc);
        }
    }
}

#[cfg(windows)]
struct DeleteDcGuard {
    hdc: HDC,
}

#[cfg(windows)]
impl Drop for DeleteDcGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.hdc);
        }
    }
}

#[cfg(windows)]
struct DeleteObjectGuard {
    handle: HGDIOBJ,
}

#[cfg(windows)]
impl Drop for DeleteObjectGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.handle);
        }
    }
}

#[cfg(windows)]
struct SelectObjectGuard {
    hdc: HDC,
    previous: HGDIOBJ,
}

#[cfg(windows)]
impl Drop for SelectObjectGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = SelectObject(self.hdc, self.previous);
        }
    }
}
