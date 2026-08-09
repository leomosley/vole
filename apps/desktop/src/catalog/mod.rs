use std::collections::HashMap;

#[cfg(windows)]
#[path = "platform_windows.rs"]
mod platform;

#[cfg(not(windows))]
#[path = "platform_fake.rs"]
mod platform;

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
    let running = platform::running_processes();
    let mut entries: HashMap<String, AppEntry> = HashMap::new();

    for (index, (executable, display)) in CURATED.iter().enumerate() {
        insert(
            &mut entries,
            executable,
            display,
            CURATED_WEIGHT - index as i32,
        );
    }

    for (executable, display) in platform::installed_applications() {
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
