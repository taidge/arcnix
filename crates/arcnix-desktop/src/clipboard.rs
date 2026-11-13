#[cfg(windows)]
pub fn copy(text: &str) -> anyhow::Result<()> {
    use windows::Win32::Foundation::{GlobalFree, HANDLE};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    anyhow::ensure!(!text.contains('\0'), "Path contains a NUL character");
    let value: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    // Clipboard ownership transfers only after SetClipboardData succeeds.
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, value.len() * 2)?;
        let buffer = GlobalLock(memory);
        if buffer.is_null() {
            let _ = GlobalFree(Some(memory));
            anyhow::bail!("Cannot allocate clipboard text");
        }
        std::ptr::copy_nonoverlapping(value.as_ptr(), buffer.cast::<u16>(), value.len());
        let _ = GlobalUnlock(memory);
        let owner = Some(GetActiveWindow());
        let mut opened = OpenClipboard(owner);
        for _ in 0..4 {
            if opened.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
            opened = OpenClipboard(owner);
        }
        if let Err(error) = opened {
            let _ = GlobalFree(Some(memory));
            return Err(error.into());
        }
        const CF_UNICODETEXT: u32 = 13;
        let result = EmptyClipboard()
            .and_then(|()| SetClipboardData(CF_UNICODETEXT, Some(HANDLE(memory.0))).map(|_| ()));
        let _ = CloseClipboard();
        if result.is_err() {
            let _ = GlobalFree(Some(memory));
        }
        result?;
    }
    Ok(())
}
#[cfg(not(windows))]
pub fn copy(_: &str) -> anyhow::Result<()> {
    anyhow::bail!("Native clipboard is currently available on Windows only")
}

#[cfg(windows)]
pub fn read(limit: usize) -> anyhow::Result<String> {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    unsafe {
        let owner = Some(GetActiveWindow());
        let mut opened = OpenClipboard(owner);
        for _ in 0..4 {
            if opened.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
            opened = OpenClipboard(owner);
        }
        opened?;
        let result = (|| -> anyhow::Result<String> {
            let handle = HGLOBAL(GetClipboardData(13)?.0);
            let capacity = GlobalSize(handle);
            anyhow::ensure!(
                capacity >= 2 && capacity <= limit.saturating_add(1).saturating_mul(2),
                "Clipboard text exceeds the terminal paste limit"
            );
            let buffer = GlobalLock(handle);
            anyhow::ensure!(!buffer.is_null(), "Cannot read clipboard text");
            let value = std::slice::from_raw_parts(buffer.cast::<u16>(), capacity / 2);
            let text = value
                .iter()
                .position(|c| *c == 0)
                .map(|end| String::from_utf16_lossy(&value[..end]));
            let _ = GlobalUnlock(handle);
            let text = text.ok_or_else(|| anyhow::anyhow!("Clipboard text is not terminated"))?;
            anyhow::ensure!(
                text.len() <= limit,
                "Clipboard text exceeds the terminal paste limit"
            );
            Ok(text)
        })();
        let _ = CloseClipboard();
        result
    }
}
#[cfg(not(windows))]
pub fn read(_: usize) -> anyhow::Result<String> {
    anyhow::bail!("Native clipboard is currently available on Windows only")
}
