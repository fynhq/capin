/// Result of applying the login-startup preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutostartState {
    Enabled,
    Disabled,
    /// macOS accepted the registration but the user must approve it in
    /// System Settings > Login Items.
    RequiresApproval,
    Unsupported,
}

#[cfg(windows)]
pub fn set_autostart(enabled: bool) -> Result<AutostartState, crate::PlatformWindowError> {
    use std::ffi::c_void;
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, KEY_READ, REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegDeleteKeyValueW,
        RegOpenKeyExW, RegQueryValueExW, RegSetKeyValueW,
    };
    use windows::core::PCWSTR;

    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const RUN_VALUE: &str = "Capin";

    fn utf16(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn startup_shortcut_path() -> Option<std::path::PathBuf> {
        std::env::var_os("APPDATA").map(|root| {
            std::path::PathBuf::from(root)
                .join("Microsoft\\Windows\\Start Menu\\Programs\\Startup\\Capin.lnk")
        })
    }

    fn remove_startup_shortcut() -> bool {
        let Some(path) = startup_shortcut_path() else {
            return true;
        };
        match std::fs::remove_file(path) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(_) => false,
        }
    }

    fn create_startup_shortcut(
        executable: &std::path::Path,
    ) -> Result<(), crate::PlatformWindowError> {
        use windows::Win32::System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            IPersistFile,
        };
        use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
        use windows::core::{Interface, PCWSTR};

        let Some(path) = startup_shortcut_path() else {
            return Err(crate::PlatformWindowError::Platform {
                operation: "startup shortcut",
                message: "APPDATA is unavailable".to_owned(),
            });
        };
        if let Some(parent) = path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            return Err(crate::PlatformWindowError::Platform {
                operation: "create startup folder",
                message: error.to_string(),
            });
        }
        let to_error = |operation: &'static str, error: windows::core::Error| {
            crate::PlatformWindowError::Platform {
                operation,
                message: error.to_string(),
            }
        };
        let shortcut = utf16(&path.to_string_lossy());
        let target = utf16(&executable.to_string_lossy());
        let working = executable
            .parent()
            .map(|directory| utf16(&directory.to_string_lossy()))
            .unwrap_or_else(|| utf16(""));
        let description = utf16("Capin");

        // The app already initialized COM; a different apartment model is not
        // fatal for creating a shell link.
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| to_error("CoCreateInstance", error))?;
            link.SetPath(PCWSTR(target.as_ptr()))
                .map_err(|error| to_error("IShellLinkW.SetPath", error))?;
            if working.len() > 1 {
                link.SetWorkingDirectory(PCWSTR(working.as_ptr()))
                    .map_err(|error| to_error("IShellLinkW.SetWorkingDirectory", error))?;
            }
            link.SetDescription(PCWSTR(description.as_ptr()))
                .map_err(|error| to_error("IShellLinkW.SetDescription", error))?;
            link.SetIconLocation(PCWSTR(target.as_ptr()), 0)
                .map_err(|error| to_error("IShellLinkW.SetIconLocation", error))?;
            let persist: IPersistFile = link
                .cast()
                .map_err(|error| to_error("IShellLinkW.cast", error))?;
            persist
                .Save(PCWSTR(shortcut.as_ptr()), true)
                .map_err(|error| to_error("IPersistFile.Save", error))?;
        }
        Ok(())
    }

    // A fixed value name is what prevents duplicate startup entries when the
    // executable moves or the app is upgraded.
    unsafe fn read_run_value() -> Option<String> {
        let key = utf16(RUN_KEY);
        let value = utf16(RUN_VALUE);
        let mut hkey = windows::Win32::System::Registry::HKEY::default();
        if unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                None,
                KEY_READ,
                &mut hkey,
            )
        }
        .is_err()
        {
            return None;
        }

        let mut size = 0u32;
        let _ = unsafe {
            RegQueryValueExW(
                hkey,
                PCWSTR(value.as_ptr()),
                None,
                None,
                None,
                Some(&mut size),
            )
        };
        if size == 0 {
            let _ = unsafe { RegCloseKey(hkey) };
            return None;
        }

        let mut data = vec![0u8; size as usize];
        let mut kind = REG_VALUE_TYPE::default();
        let queried = unsafe {
            RegQueryValueExW(
                hkey,
                PCWSTR(value.as_ptr()),
                None,
                Some(&mut kind),
                Some(data.as_mut_ptr()),
                Some(&mut size),
            )
        };
        let _ = unsafe { RegCloseKey(hkey) };
        if queried.is_err() || kind != REG_SZ {
            return None;
        }

        let wide: Vec<u16> = data[..size as usize]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .take_while(|unit| *unit != 0)
            .collect();
        String::from_utf16(&wide).ok()
    }

    if enabled {
        let executable =
            std::env::current_exe().map_err(|error| crate::PlatformWindowError::Platform {
                operation: "current_exe",
                message: error.to_string(),
            })?;
        let command = format!("\"{}\"", executable.display());
        if unsafe { read_run_value() }.as_deref() == Some(command.as_str()) {
            let _ = remove_startup_shortcut();
            return Ok(AutostartState::Enabled);
        }

        // A previous fallback entry is already sufficient; do not rewrite it
        // on every launch when the Run key stays blocked.
        if startup_shortcut_path().is_some_and(|path| path.exists()) {
            return Ok(AutostartState::Enabled);
        }

        let key = utf16(RUN_KEY);
        let value = utf16(RUN_VALUE);
        let data: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
        let result = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR(value.as_ptr()),
                REG_SZ.0,
                Some(data.as_ptr().cast::<c_void>()),
                (data.len() * 2) as u32,
            )
        };
        if result.is_ok() {
            let _ = remove_startup_shortcut();
            return Ok(AutostartState::Enabled);
        }

        // Some managed machines deny writes to the Run key. Drop a stale value
        // if allowed, then use a Startup folder shortcut, which needs no
        // elevation.
        let _ = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR(value.as_ptr()),
            )
        };
        match create_startup_shortcut(&executable) {
            Ok(()) => Ok(AutostartState::Enabled),
            Err(shortcut_error) => Err(crate::PlatformWindowError::Platform {
                operation: "RegSetKeyValueW",
                message: format!(
                    "error {}; startup shortcut failed: {shortcut_error}",
                    result.0
                ),
            }),
        }
    } else {
        let mut error = None;
        if unsafe { read_run_value() }.is_some() {
            let key = utf16(RUN_KEY);
            let value = utf16(RUN_VALUE);
            let result = unsafe {
                RegDeleteKeyValueW(
                    HKEY_CURRENT_USER,
                    PCWSTR(key.as_ptr()),
                    PCWSTR(value.as_ptr()),
                )
            };
            // ERROR_FILE_NOT_FOUND means another process removed it first.
            if !result.is_ok() && result.0 != 2 {
                error = Some(crate::PlatformWindowError::Platform {
                    operation: "RegDeleteKeyValueW",
                    message: format!("error {}", result.0),
                });
            }
        }
        if !remove_startup_shortcut() && error.is_none() {
            error = Some(crate::PlatformWindowError::Platform {
                operation: "startup shortcut",
                message: "failed to remove the shortcut".to_owned(),
            });
        }
        match error {
            Some(error) => Err(error),
            None => Ok(AutostartState::Disabled),
        }
    }
}

#[cfg(target_os = "macos")]
pub fn set_autostart(enabled: bool) -> Result<AutostartState, crate::PlatformWindowError> {
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::{msg_send, sel};

    // SMAppService lives in ServiceManagement.framework. Login items are keyed
    // by code-signing identity, so packaging must ad-hoc sign the bundle with
    // a fixed identifier or every build creates another login item.
    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    let class =
        AnyClass::get(c"SMAppService").ok_or_else(|| crate::PlatformWindowError::Platform {
            operation: "SMAppService",
            message: "requires macOS 13".to_owned(),
        })?;
    if class.class_method(sel!(mainAppService)).is_none() {
        return Err(crate::PlatformWindowError::Platform {
            operation: "SMAppService",
            message: "mainAppService selector is unavailable".to_owned(),
        });
    }

    let service: *mut AnyObject = unsafe { msg_send![class, mainAppService] };
    if service.is_null() {
        return Err(crate::PlatformWindowError::Platform {
            operation: "SMAppService",
            message: "main app service is unavailable".to_owned(),
        });
    }

    let status: isize = unsafe { msg_send![service, status] };
    if enabled {
        if status == 1 {
            return Ok(AutostartState::Enabled);
        }
        if status == 2 {
            return Ok(AutostartState::RequiresApproval);
        }
    } else if status == 0 || status == 3 {
        return Ok(AutostartState::Disabled);
    }

    let mut error: *mut AnyObject = std::ptr::null_mut();
    let ok: Bool = unsafe {
        if enabled {
            msg_send![service, registerAndReturnError: &mut error]
        } else {
            msg_send![service, unregisterAndReturnError: &mut error]
        }
    };
    let new_status: isize = unsafe { msg_send![service, status] };
    if !ok.as_bool() {
        return Err(crate::PlatformWindowError::Platform {
            operation: "SMAppService",
            message: format!("status={new_status}"),
        });
    }

    if enabled {
        if new_status == 2 {
            Ok(AutostartState::RequiresApproval)
        } else {
            Ok(AutostartState::Enabled)
        }
    } else {
        Ok(AutostartState::Disabled)
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn set_autostart(_enabled: bool) -> Result<AutostartState, crate::PlatformWindowError> {
    Ok(AutostartState::Unsupported)
}
