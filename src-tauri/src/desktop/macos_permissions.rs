//! Permission APIs must be optional at runtime: Catalina does not export all
//! CoreGraphics permission functions, even when the SDK marks them as available.
use std::ffi::CStr;

type PermissionFunction = unsafe extern "C" fn() -> bool;

pub(super) struct PermissionApi {
    post_event_access: Option<PermissionFunction>,
    screen_capture_access: Option<PermissionFunction>,
    request_post_event_access: Option<PermissionFunction>,
    request_screen_capture_access: Option<PermissionFunction>,
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    pub(super) fn AXIsProcessTrusted() -> bool;
    pub(super) fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> bool;
}

impl PermissionApi {
    pub(super) fn load() -> Self {
        Self::resolve(resolve_function)
    }

    fn resolve(mut resolve: impl FnMut(&CStr) -> Option<PermissionFunction>) -> Self {
        Self {
            post_event_access: resolve(c"CGPreflightPostEventAccess"),
            screen_capture_access: resolve(c"CGPreflightScreenCaptureAccess"),
            request_post_event_access: resolve(c"CGRequestPostEventAccess"),
            request_screen_capture_access: resolve(c"CGRequestScreenCaptureAccess"),
        }
    }

    pub(super) fn accessibility_granted(&self, trusted: bool) -> bool {
        // Input Monitoring (listen-event access) does not grant desktop control.
        trusted || call_permission(self.post_event_access).unwrap_or(false)
    }

    pub(super) fn screen_capture_granted(&self) -> Option<bool> {
        call_permission(self.screen_capture_access)
    }

    pub(super) fn request_screen_capture(&self) -> Result<(), String> {
        call_permission(self.request_screen_capture_access)
            .map(|_| ())
            .ok_or_else(|| {
                "当前系统无法自动申请屏幕录制权限，请在系统隐私设置中确认授权".to_string()
            })
    }

    pub(super) fn request_post_event(&self) {
        // AXIsProcessTrustedWithOptions remains the accessibility prompt on
        // systems without this additional event-posting API.
        let _ = call_permission(self.request_post_event_access);
    }
}

fn call_permission(function: Option<PermissionFunction>) -> Option<bool> {
    function.map(|function| unsafe { function() })
}

fn resolve_function(name: &CStr) -> Option<PermissionFunction> {
    let address = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    if address.is_null() {
        None
    } else {
        // Every caller supplies a fixed CoreGraphics symbol with this exact ABI.
        Some(unsafe { std::mem::transmute::<*mut std::ffi::c_void, PermissionFunction>(address) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "C" fn allowed() -> bool {
        true
    }

    unsafe extern "C" fn denied() -> bool {
        false
    }

    #[test]
    fn missing_apis_do_not_crash_or_claim_authorization() {
        let api = PermissionApi::resolve(|_| None);
        assert!(!api.accessibility_granted(false));
        assert!(api.accessibility_granted(true));
        assert_eq!(api.screen_capture_granted(), None);
        assert!(api.request_screen_capture().is_err());
        api.request_post_event();
    }

    #[test]
    fn available_apis_distinguish_denial_from_missing_support() {
        let api = PermissionApi::resolve(|_| Some(denied));
        assert!(!api.accessibility_granted(false));
        assert_eq!(api.screen_capture_granted(), Some(false));
        assert!(api.request_screen_capture().is_ok());
        let api = PermissionApi::resolve(|_| Some(allowed));
        assert!(api.accessibility_granted(false));
        assert_eq!(api.screen_capture_granted(), Some(true));
    }

    #[test]
    fn catalina_can_use_accessibility_without_screen_permission_apis() {
        let api = PermissionApi::resolve(|name| match name.to_bytes() {
            b"CGPreflightPostEventAccess" => Some(allowed),
            _ => None,
        });
        assert!(api.accessibility_granted(false));
        assert_eq!(api.screen_capture_granted(), None);
        assert!(api.request_screen_capture().is_err());
    }

    #[test]
    fn input_monitoring_is_not_used_as_desktop_control_authorization() {
        let api = PermissionApi::resolve(|name| {
            assert_ne!(name.to_bytes(), b"CGPreflightListenEventAccess");
            None
        });
        assert!(!api.accessibility_granted(false));
    }

    #[test]
    fn real_loader_handles_an_absent_symbol() {
        assert!(resolve_function(c"BaijimuMissingPermissionSymbolForTest").is_none());
    }
}
