//! Windows: `IPolicyConfig`, the undocumented COM interface that sets the default output.

#![expect(non_snake_case, reason = "COM method names")]
#![expect(clippy::transmute_ptr_to_ptr, reason = "inside the windows interface macro's expansion")]

use windows::core::PCWSTR;
use windows::Win32::Media::Audio::ERole;

/// Windows' own way to pick the default output; not in the public SDK (the volume mixers that switch outputs,
/// like `EarTrumpet` and `SoundSwitch`, use it the same way). Only the methods up to `SetDefaultEndpoint` are called.
#[windows_core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
pub unsafe trait IPolicyConfig: windows::core::IUnknown {
    fn GetMixFormat(&self, device: PCWSTR, format: *mut *mut std::ffi::c_void) -> windows::core::HRESULT;
    fn GetDeviceFormat(
        &self,
        device: PCWSTR,
        default: i32,
        format: *mut *mut std::ffi::c_void,
    ) -> windows::core::HRESULT;
    fn ResetDeviceFormat(&self, device: PCWSTR) -> windows::core::HRESULT;
    fn SetDeviceFormat(
        &self,
        device: PCWSTR,
        endpoint: *mut std::ffi::c_void,
        mix: *mut std::ffi::c_void,
    ) -> windows::core::HRESULT;
    fn GetProcessingPeriod(&self, device: PCWSTR, default: i32, a: *mut i64, b: *mut i64) -> windows::core::HRESULT;
    fn SetProcessingPeriod(&self, device: PCWSTR, period: *mut i64) -> windows::core::HRESULT;
    fn GetShareMode(&self, device: PCWSTR, mode: *mut std::ffi::c_void) -> windows::core::HRESULT;
    fn SetShareMode(&self, device: PCWSTR, mode: *mut std::ffi::c_void) -> windows::core::HRESULT;
    fn GetPropertyValue(
        &self,
        device: PCWSTR,
        key: *const std::ffi::c_void,
        value: *mut std::ffi::c_void,
    ) -> windows::core::HRESULT;
    fn SetPropertyValue(
        &self,
        device: PCWSTR,
        key: *const std::ffi::c_void,
        value: *mut std::ffi::c_void,
    ) -> windows::core::HRESULT;
    pub fn SetDefaultEndpoint(&self, device: PCWSTR, role: ERole) -> windows::core::HRESULT;
}
