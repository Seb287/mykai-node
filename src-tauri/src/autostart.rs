/// Windows auto-start management via registry.
/// Sets or removes the app from HKCU\Software\Microsoft\Windows\CurrentVersion\Run
/// so it launches automatically when the user logs in.

#[cfg(windows)]
pub fn set_auto_start(enabled: bool) -> Result<(), String> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run_key = hkcu
        .open_subkey_with_flags(
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            KEY_SET_VALUE | KEY_READ,
        )
        .map_err(|e| format!("Failed to open registry key: {}", e))?;

    let app_name = "MyKAI Node";

    if enabled {
        // Get current executable path
        let exe_path = std::env::current_exe()
            .map_err(|e| format!("Failed to get executable path: {}", e))?;

        // M-9: Use .to_str() for Unicode safety instead of .to_string_lossy()
        let exe_str = exe_path
            .to_str()
            .ok_or_else(|| "Executable path contains invalid Unicode characters".to_string())?;

        // M-9: Quote the path to handle spaces (e.g. "C:\Users\John Smith\...")
        let quoted = format!("\"{}\"", exe_str);

        run_key
            .set_value(app_name, &quoted)
            .map_err(|e| format!("Failed to set registry value: {}", e))?;

        tracing::info!("Auto-start enabled: {}", exe_path.display());
    } else {
        // Remove the registry entry (ignore error if it doesn't exist)
        let _ = run_key.delete_value(app_name);
        tracing::info!("Auto-start disabled");
    }

    Ok(())
}

/// No-op on non-Windows platforms.
#[cfg(not(windows))]
pub fn set_auto_start(enabled: bool) -> Result<(), String> {
    if enabled {
        Err("Auto-start is only supported on Windows. Use systemd or launchd on your platform.".to_string())
    } else {
        Ok(())
    }
}
