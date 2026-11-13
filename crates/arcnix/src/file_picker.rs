//! Native file selection; returns a path without opening the selected key.
use std::path::{Path, PathBuf};

pub(super) fn field(ui: super::Ui) -> impl glory::prelude::Widget {
    use glory::prelude::*;

    use super::{AuthMethod, Dialog, i18n};
    let state = ui.state;
    col().class("field").fill((
        text()
            .class("field-label")
            .content(i18n::bind("私钥文件路径")),
        row().class("file-picker-field").fill((
            input()
                .name(i18n::bind("私钥文件路径"))
                .value(state.map(|s| {
                    s.editor
                        .private_key
                        .as_ref()
                        .map(|path| super::local_display(path))
                        .unwrap_or_default()
                }))
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        state.revise(|s| {
                            s.editor.private_key = if value.trim().is_empty() {
                                None
                            } else {
                                Some(value.into())
                            };
                            s.dialog_error.clear();
                        });
                    }
                }),
            sapid::button()
                .class("button browse-button")
                .label(i18n::bind("Browse…"))
                .on_press(move || {
                    let (current, id) =
                        state.with(|s| (s.editor.private_key.clone(), s.editor.id.clone()));
                    match private_key(
                        current.as_deref(),
                        &i18n::translate("Select SSH private key"),
                        &i18n::translate("Select file"),
                        &i18n::translate("All files"),
                    ) {
                        Ok(Some(path)) => state.revise(|s| {
                            if s.dialog == Some(Dialog::Server)
                                && s.editor.id == id
                                && s.editor.auth == AuthMethod::PrivateKey
                            {
                                s.editor.private_key = Some(path);
                                s.dialog_error.clear();
                            }
                        }),
                        Ok(None) => {}
                        Err(error) => state.revise(|s| {
                            s.dialog_error = format!("Could not select private key: {error:#}")
                        }),
                    }
                }),
        )),
    ))
}

pub(super) fn initial_directory(current: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = current {
        if path.is_dir() {
            return Some(path.to_path_buf());
        }
        if let Some(parent) = path.parent().filter(|parent| parent.is_dir()) {
            return Some(parent.to_path_buf());
        }
    }
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|home| home.join(".ssh"))
        .filter(|path| path.is_dir())
}

#[cfg(windows)]
pub(super) fn private_key(
    current: Option<&Path>,
    title: &str,
    choose: &str,
    all_files: &str,
) -> anyhow::Result<Option<PathBuf>> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    use anyhow::{Context, ensure};
    use windows::Win32::Foundation::ERROR_CANCELLED;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FOS_DONTADDTORECENT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR,
        FOS_PATHMUSTEXIST, FileOpenDialog, IFileOpenDialog, IShellItem,
        SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor};
    use windows::core::{HRESULT, HSTRING, w};
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    // Called on the UI thread. The owner disables the application while Windows
    // runs its modal loop, so the result cannot land in a different server form.
    unsafe {
        let active = GetActiveWindow();
        ensure!(
            !active.is_invalid(),
            "Cannot find the application window for file selection"
        );
        let owner = GetAncestor(active, GA_ROOT);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .context("Cannot initialize the file dialog")?;
        let _apartment = Apartment;
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetOptions(
            dialog.GetOptions()?
                | FOS_FILEMUSTEXIST
                | FOS_PATHMUSTEXIST
                | FOS_FORCEFILESYSTEM
                | FOS_DONTADDTORECENT
                | FOS_NOCHANGEDIR,
        )?;
        dialog.SetTitle(&HSTRING::from(title))?;
        dialog.SetOkButtonLabel(&HSTRING::from(choose))?;
        let filter_name = HSTRING::from(all_files);
        dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
            pszName: windows::core::PCWSTR(filter_name.as_ptr()),
            pszSpec: w!("*"),
        }])?;
        if let Some(directory) = initial_directory(current) {
            let directory = HSTRING::from(directory.as_os_str());
            if let Ok(folder) = SHCreateItemFromParsingName::<_, _, IShellItem>(&directory, None) {
                let _ = dialog.SetFolder(&folder);
            }
        }
        match dialog.Show(Some(owner)) {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => {
                return Ok(None);
            }
            Err(error) => return Err(error).context("Cannot open the file selection window"),
        }
        let item = dialog.GetResult()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = PathBuf::from(OsString::from_wide(name.as_wide()));
        CoTaskMemFree(Some(name.0.cast()));
        ensure!(path.is_file(), "Please select an existing file");
        Ok(Some(path))
    }
}

#[cfg(not(windows))]
pub(super) fn private_key(
    _current: Option<&Path>,
    _title: &str,
    _choose: &str,
    _all_files: &str,
) -> anyhow::Result<Option<PathBuf>> {
    anyhow::bail!(
        "Native file selection is currently supported on Windows. Enter the path manually."
    )
}
