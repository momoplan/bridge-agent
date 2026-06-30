use anyhow::Result;

#[derive(Debug)]
pub struct SystemSleepPrevention {
    platform: PlatformAssertion,
}

impl SystemSleepPrevention {
    pub fn acquire(reason: &str) -> Result<Self> {
        Ok(Self {
            platform: PlatformAssertion::acquire(reason)?,
        })
    }

    pub fn is_active(&self) -> bool {
        self.platform.is_active()
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct PlatformAssertion {
    id: u32,
}

#[cfg(target_os = "macos")]
impl PlatformAssertion {
    fn acquire(reason: &str) -> Result<Self> {
        macos::acquire_system_sleep_prevention(reason)
    }

    fn is_active(&self) -> bool {
        self.id != 0
    }
}

#[cfg(target_os = "macos")]
impl Drop for PlatformAssertion {
    fn drop(&mut self) {
        if self.id != 0 {
            unsafe {
                macos::IOPMAssertionRelease(self.id);
            }
            self.id = 0;
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::PlatformAssertion;
    use anyhow::{bail, Result};
    use std::ffi::{c_char, c_void, CString};
    use std::ptr;

    type CFAllocatorRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFTypeRef = *const c_void;
    type IOReturn = i32;
    type IOPMAssertionID = u32;
    type IOPMAssertionLevel = u32;

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const K_IOPM_ASSERTION_LEVEL_ON: IOPMAssertionLevel = 255;
    const K_IO_RETURN_SUCCESS: IOReturn = 0;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            alloc: CFAllocatorRef,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: CFTypeRef);
    }

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: CFStringRef,
            assertion_level: IOPMAssertionLevel,
            assertion_name: CFStringRef,
            assertion_id: *mut IOPMAssertionID,
        ) -> IOReturn;
        pub(super) fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> IOReturn;
    }

    pub(super) fn acquire_system_sleep_prevention(reason: &str) -> Result<PlatformAssertion> {
        let assertion_type = cf_string("PreventUserIdleSystemSleep")?;
        let assertion_name = cf_string(reason)?;
        let mut assertion_id = 0;
        let result = unsafe {
            IOPMAssertionCreateWithName(
                assertion_type.as_ref(),
                K_IOPM_ASSERTION_LEVEL_ON,
                assertion_name.as_ref(),
                &mut assertion_id,
            )
        };

        if result != K_IO_RETURN_SUCCESS {
            bail!("IOPMAssertionCreateWithName failed with IOReturn {result}");
        }
        if assertion_id == 0 {
            bail!("IOPMAssertionCreateWithName returned an empty assertion id");
        }

        Ok(PlatformAssertion { id: assertion_id })
    }

    struct CfString {
        value: CFStringRef,
    }

    impl CfString {
        fn as_ref(&self) -> CFStringRef {
            self.value
        }
    }

    impl Drop for CfString {
        fn drop(&mut self) {
            if !self.value.is_null() {
                unsafe {
                    CFRelease(self.value);
                }
                self.value = ptr::null();
            }
        }
    }

    fn cf_string(value: &str) -> Result<CfString> {
        let c_value = CString::new(value)?;
        let cf_value = unsafe {
            CFStringCreateWithCString(ptr::null(), c_value.as_ptr(), K_CF_STRING_ENCODING_UTF8)
        };
        if cf_value.is_null() {
            bail!("failed to create CFString");
        }
        Ok(CfString { value: cf_value })
    }
}

#[cfg(not(target_os = "macos"))]
#[derive(Debug)]
struct PlatformAssertion;

#[cfg(not(target_os = "macos"))]
impl PlatformAssertion {
    fn acquire(_reason: &str) -> Result<Self> {
        Ok(Self)
    }

    fn is_active(&self) -> bool {
        false
    }
}
