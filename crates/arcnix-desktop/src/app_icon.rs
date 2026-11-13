//! Native window icons share resource 1 with the executable's Explorer icon.
#[cfg(windows)]
pub struct AppIcon;

#[cfg(windows)]
impl glory::wry::shell::NativeLifecycle for AppIcon {
    fn opened(
        &mut self,
        window: &glory::wry::shell::NativeWindow,
        _wake: &std::task::Waker,
    ) -> Result<(), glory::wry::shell::ShellError> {
        use glory::wry::shell::NativeWindowExtWindows;
        use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_SHARED, LoadImageW, SM_CXICON,
            SM_CXSMICON, SM_CYICON, SM_CYSMICON, SendMessageW, WM_SETICON,
        };
        use windows::core::PCWSTR;

        // LR_SHARED gives the module ownership of these resource-backed handles.
        unsafe {
            let instance = GetModuleHandleW(None)
                .map_err(|error| glory::wry::shell::ShellError(error.to_string()))?;
            for (kind, width, height) in [
                (ICON_SMALL, SM_CXSMICON, SM_CYSMICON),
                (ICON_BIG, SM_CXICON, SM_CYICON),
            ] {
                let icon = LoadImageW(
                    Some(HINSTANCE(instance.0)),
                    PCWSTR(std::ptr::without_provenance(1)),
                    IMAGE_ICON,
                    GetSystemMetrics(width),
                    GetSystemMetrics(height),
                    LR_SHARED,
                )
                .map_err(|error| glory::wry::shell::ShellError(error.to_string()))?;
                SendMessageW(
                    HWND(window.hwnd() as *mut _),
                    WM_SETICON,
                    Some(WPARAM(kind as usize)),
                    Some(LPARAM(icon.0 as isize)),
                );
            }
        }
        Ok(())
    }
}
