#![expect(unsafe_code, reason = "Windows process and shell APIs require FFI")]

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::UI::Shell::{IShellLinkW, SLGP_RAWPATH, ShellLink};
use windows::core::{Interface, PCWSTR};

pub fn running_processes() -> HashSet<String> {
    let mut names = HashSet::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return names;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let name = wide_to_string(&entry.szExeFile);
                if !name.is_empty() {
                    names.insert(name.to_ascii_lowercase());
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    names
}

pub fn installed_applications() -> Vec<(String, String)> {
    let mut shortcuts = Vec::new();
    for directory in start_menu_directories() {
        collect_shortcuts(&directory, &mut shortcuts);
    }

    let mut applications = Vec::new();
    unsafe {
        let Ok(link) = CoCreateInstance::<_, IShellLinkW>(&ShellLink, None, CLSCTX_INPROC_SERVER)
        else {
            return applications;
        };
        let Ok(persist) = link.cast::<IPersistFile>() else {
            return applications;
        };

        for shortcut in shortcuts {
            let Some(display) = shortcut
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned)
            else {
                continue;
            };
            let wide = to_wide(&shortcut.to_string_lossy());
            if persist.Load(PCWSTR(wide.as_ptr()), STGM_READ).is_err() {
                continue;
            }
            let mut buffer = [0_u16; 260];
            let mut find = WIN32_FIND_DATAW::default();
            if link
                .GetPath(&mut buffer, &mut find, SLGP_RAWPATH.0 as u32)
                .is_err()
            {
                continue;
            }
            if let Some(executable) = executable_name(&wide_to_string(&buffer)) {
                applications.push((executable, display));
            }
        }
    }
    applications
}

fn start_menu_directories() -> Vec<PathBuf> {
    ["ProgramData", "APPDATA"]
        .iter()
        .filter_map(|variable| std::env::var(variable).ok())
        .map(|base| PathBuf::from(base).join(r"Microsoft\Windows\Start Menu\Programs"))
        .collect()
}

fn collect_shortcuts(directory: &Path, shortcuts: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_shortcuts(&path, shortcuts);
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
        {
            shortcuts.push(path);
        }
    }
}

fn executable_name(target: &str) -> Option<String> {
    let name = Path::new(target).file_name()?.to_str()?;
    name.to_ascii_lowercase()
        .ends_with(".exe")
        .then(|| name.to_owned())
}

fn wide_to_string(buffer: &[u16]) -> String {
    let length = buffer
        .iter()
        .position(|&code| code == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}

fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
