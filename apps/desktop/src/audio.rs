#![expect(
    unsafe_code,
    reason = "Windows Core Audio and process APIs require FFI"
)]

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Media::Audio::{
    IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume,
    MMDeviceEnumerator, eMultimedia, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::core::{Interface, PWSTR};

use crate::config::{HotkeyBinding, Operation, Target};

pub struct AudioController {
    _com: ComApartment,
    devices: IMMDeviceEnumerator,
    toggles: HashMap<String, HashMap<u32, SessionState>>,
}

impl AudioController {
    pub fn new() -> Result<Self> {
        let com = ComApartment::initialize()?;
        let devices = unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER)
                .context("failed to create Windows audio device enumerator")?
        };

        Ok(Self {
            _com: com,
            devices,
            toggles: HashMap::new(),
        })
    }

    pub fn apply(&mut self, binding: &HotkeyBinding) -> Result<()> {
        let foreground_pid = foreground_pid();
        let sessions = self.sessions()?;

        if binding.toggle && self.restore_toggle(binding, &sessions)? {
            return Ok(());
        }

        for action in &binding.actions {
            for session in &sessions {
                if session.matches(&action.target, foreground_pid) {
                    session.apply(action.operation)?;
                }
            }
        }

        Ok(())
    }

    fn restore_toggle(
        &mut self,
        binding: &HotkeyBinding,
        sessions: &[AudioSession],
    ) -> Result<bool> {
        if let Some(snapshot) = self.toggles.remove(&binding.id) {
            for session in sessions {
                if let Some(state) = snapshot.get(&session.pid) {
                    session.restore(*state)?;
                }
            }
            return Ok(true);
        }

        let foreground_pid = foreground_pid();
        let mut snapshot = HashMap::new();
        for action in &binding.actions {
            for session in sessions {
                if session.matches(&action.target, foreground_pid)
                    && !snapshot.contains_key(&session.pid)
                {
                    snapshot.insert(session.pid, session.capture()?);
                }
            }
        }
        self.toggles.insert(binding.id.clone(), snapshot);
        Ok(false)
    }

    pub fn applications(&self) -> Result<Vec<String>> {
        let mut executables = self
            .sessions()?
            .into_iter()
            .filter_map(|session| session.executable)
            .collect::<Vec<_>>();
        executables.sort_unstable_by_key(|executable| executable.to_ascii_lowercase());
        executables.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        Ok(executables)
    }

    fn sessions(&self) -> Result<Vec<AudioSession>> {
        let device = unsafe {
            self.devices
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .context("failed to get default Windows audio output")?
        };
        let manager: IAudioSessionManager2 = unsafe {
            device
                .Activate(CLSCTX_INPROC_SERVER, None)
                .context("failed to activate Windows audio session manager")?
        };
        let enumerator = unsafe {
            manager
                .GetSessionEnumerator()
                .context("failed to enumerate Windows audio sessions")?
        };
        let count = unsafe { enumerator.GetCount() }.context("failed to count audio sessions")?;
        let mut sessions = Vec::with_capacity(count.max(0) as usize);

        for index in 0..count {
            let control = unsafe { enumerator.GetSession(index) }
                .with_context(|| format!("failed to get audio session {index}"))?;
            let control2: IAudioSessionControl2 = control
                .cast()
                .with_context(|| format!("failed to inspect audio session {index}"))?;
            let pid = unsafe { control2.GetProcessId() }
                .with_context(|| format!("failed to get process for audio session {index}"))?;
            if pid == 0 {
                continue;
            }
            let volume: ISimpleAudioVolume = control
                .cast()
                .with_context(|| format!("failed to control audio session {index}"))?;
            sessions.push(AudioSession {
                pid,
                executable: process_name(pid),
                volume,
            });
        }

        Ok(sessions)
    }
}

struct AudioSession {
    pid: u32,
    executable: Option<String>,
    volume: ISimpleAudioVolume,
}

impl AudioSession {
    fn matches(&self, target: &Target, foreground_pid: Option<u32>) -> bool {
        match target {
            Target::Foreground => foreground_pid == Some(self.pid),
            Target::Process { executable } => self
                .executable
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(executable)),
        }
    }

    fn apply(&self, operation: Operation) -> Result<()> {
        match operation {
            Operation::Set { level } => self.set_level(level),
            Operation::Adjust { delta } => self.adjust(delta),
            Operation::Mute { muted } => self.set_mute(muted),
        }
    }

    fn capture(&self) -> Result<SessionState> {
        Ok(SessionState {
            level: self.level()?,
            muted: self.muted()?,
        })
    }

    fn restore(&self, state: SessionState) -> Result<()> {
        self.set_level(state.level)?;
        self.set_mute(state.muted)
    }

    fn level(&self) -> Result<f32> {
        unsafe { self.volume.GetMasterVolume() }.context("failed to read application volume")
    }

    fn set_level(&self, level: f32) -> Result<()> {
        unsafe {
            self.volume
                .SetMasterVolume(level.clamp(0.0, 1.0), std::ptr::null())
        }
        .context("failed to set application volume")
    }

    fn adjust(&self, delta: f32) -> Result<()> {
        self.set_level(self.level()? + delta)
    }

    fn muted(&self) -> Result<bool> {
        Ok(unsafe { self.volume.GetMute() }
            .context("failed to read application mute state")?
            .as_bool())
    }

    fn set_mute(&self, muted: bool) -> Result<()> {
        unsafe { self.volume.SetMute(muted, std::ptr::null()) }
            .context("failed to set application mute state")
    }
}

#[derive(Clone, Copy)]
struct SessionState {
    level: f32,
    muted: bool,
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .context("failed to initialize Windows COM")?;
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct ProcessHandle(HANDLE);

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn foreground_pid() -> Option<u32> {
    unsafe {
        let window = GetForegroundWindow();
        let mut pid = 0;
        GetWindowThreadProcessId(window, Some(&mut pid));
        (pid != 0).then_some(pid)
    }
}

fn process_name(pid: u32) -> Option<String> {
    let path = process_path(pid).ok()?;
    Path::new(&path).file_name()?.to_str().map(str::to_owned)
}

fn process_path(pid: u32) -> windows::core::Result<String> {
    unsafe {
        let process = ProcessHandle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?);
        let mut buffer = vec![0_u16; 32_768];
        let mut length = buffer.len() as u32;
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )?;
        Ok(String::from_utf16_lossy(&buffer[..length as usize]))
    }
}
