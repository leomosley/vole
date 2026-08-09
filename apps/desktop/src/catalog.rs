#![expect(unsafe_code, reason = "Windows process and shell APIs require FFI")]

use std::collections::HashMap;
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

// Common applications surfaced at the top of the picker, highest weight first.
const CURATED: &[(&str, &str)] = &[
    ("Discord.exe", "Discord"),
    ("chrome.exe", "Google Chrome"),
    ("msedge.exe", "Microsoft Edge"),
    ("firefox.exe", "Mozilla Firefox"),
    ("Spotify.exe", "Spotify"),
    ("slack.exe", "Slack"),
    ("ms-teams.exe", "Microsoft Teams"),
    ("Teams.exe", "Microsoft Teams"),
    ("vlc.exe", "VLC media player"),
    ("obs64.exe", "OBS Studio"),
    ("Zoom.exe", "Zoom"),
    ("Telegram.exe", "Telegram"),
    ("WhatsApp.exe", "WhatsApp"),
    ("steam.exe", "Steam"),
    ("EpicGamesLauncher.exe", "Epic Games Launcher"),
    ("Battle.net.exe", "Battle.net"),
    ("RobloxPlayerBeta.exe", "Roblox"),
    ("VALORANT-Win64-Shipping.exe", "VALORANT"),
    ("LeagueClient.exe", "League of Legends"),
    ("League of Legends.exe", "League of Legends (game)"),
    ("cs2.exe", "Counter-Strike 2"),
    ("FortniteClient-Win64-Shipping.exe", "Fortnite"),
    ("Overwatch.exe", "Overwatch"),
    ("dota2.exe", "Dota 2"),
    ("javaw.exe", "Minecraft (Java)"),
    ("Minecraft.Windows.exe", "Minecraft"),
    ("bg3.exe", "Baldur's Gate 3"),
    ("eldenring.exe", "Elden Ring"),
    ("GTA5.exe", "Grand Theft Auto V"),
    ("Wow.exe", "World of Warcraft"),
];

const CURATED_WEIGHT: i32 = 1_000_000;
const RUNNING_WEIGHT: i32 = 1_000;

#[derive(Clone, Debug)]
pub struct AppEntry {
    pub executable: String,
    pub display: String,
    pub running: bool,
    pub weight: i32,
}

// Builds the application catalogue: curated apps, installed Start Menu apps,
// and anything currently producing audio, ordered so the most relevant appear first.
#[must_use]
pub fn build(audio_playing: &[String]) -> Vec<AppEntry> {
    let running = running_processes();
    let mut entries: HashMap<String, AppEntry> = HashMap::new();

    for (index, (executable, display)) in CURATED.iter().enumerate() {
        insert(
            &mut entries,
            executable,
            display,
            CURATED_WEIGHT - index as i32,
        );
    }

    for (executable, display) in installed_applications() {
        insert(&mut entries, &executable, &display, 0);
    }

    for executable in audio_playing {
        insert(&mut entries, executable, &derive_display(executable), 0);
    }

    for entry in entries.values_mut() {
        if running.contains(&entry.executable.to_ascii_lowercase())
            || audio_playing
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&entry.executable))
        {
            entry.running = true;
            entry.weight += RUNNING_WEIGHT;
        }
    }

    let mut catalog = entries.into_values().collect::<Vec<_>>();
    catalog.sort_by(|left, right| {
        right.weight.cmp(&left.weight).then_with(|| {
            left.display
                .to_ascii_lowercase()
                .cmp(&right.display.to_ascii_lowercase())
        })
    });
    catalog
}

fn insert(entries: &mut HashMap<String, AppEntry>, executable: &str, display: &str, weight: i32) {
    let key = executable.to_ascii_lowercase();
    entries.entry(key).or_insert_with(|| AppEntry {
        executable: executable.to_owned(),
        display: display.to_owned(),
        running: false,
        weight,
    });
}

fn derive_display(executable: &str) -> String {
    executable
        .strip_suffix(".exe")
        .or_else(|| executable.strip_suffix(".EXE"))
        .unwrap_or(executable)
        .to_owned()
}

fn running_processes() -> HashSet<String> {
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

fn installed_applications() -> Vec<(String, String)> {
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
