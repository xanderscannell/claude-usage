//! The Macros page: buttons read from `macros.json` in the app's config
//! folder (`%APPDATA%\dev.xander.claude-usage`). The page only ever sends
//! a button's position, so it can run what the file says and nothing else.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Written on first run, so there is a working file to edit.
const STARTER: &str = r##"{
  "columns": 5,
  "buttons": [
    { "label": "Previous", "icon": "⏮", "media": "previous" },
    { "label": "Play / Pause", "icon": "⏯", "media": "play_pause", "color": "#55A87A" },
    { "label": "Next", "icon": "⏭", "media": "next" },
    { "label": "Volume −", "icon": "−", "media": "volume_down" },
    { "label": "Volume +", "icon": "+", "media": "volume_up" },
    { "label": "Mute", "icon": "×", "media": "mute" },
    { "label": "Snip", "icon": "✂", "keys": "win+shift+s", "color": "#5B8FDD" },
    { "label": "Task Manager", "icon": "▦", "keys": "ctrl+shift+esc" },
    { "label": "Claude", "icon": "✳", "open": "https://claude.ai", "color": "#C2513A" },
    { "label": "Copy date", "icon": "▤", "run": "Get-Date -Format 'yyyy-MM-dd' | Set-Clipboard", "color": "#9B7BD4" },
    { "label": "Desktop", "icon": "▭", "keys": "win+d" },
    { "label": "Explorer", "icon": "◫", "keys": "win+e", "color": "#D0A85E" },
    { "label": "Clipboard", "icon": "▣", "keys": "win+v" },
    { "label": "Calculator", "icon": "±", "open": "calc.exe" },
    { "label": "Settings", "icon": "⚙", "keys": "win+i" }
  ]
}
"##;

#[derive(Deserialize)]
struct File {
    #[serde(default = "five")]
    columns: u32,
    buttons: Vec<Button>,
}

fn five() -> u32 {
    5
}

/// One button. Exactly one of `media`, `keys`, `open` or `run` says what it does.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Button {
    label: String,
    icon: Option<String>,
    color: Option<String>,
    media: Option<String>,
    keys: Option<String>,
    open: Option<String>,
    /// Arguments for `open`, when it names a program.
    args: Option<String>,
    run: Option<String>,
}

#[derive(Debug)]
enum Action {
    /// Virtual-key codes, pressed in order and released in reverse.
    Keys(Vec<u16>),
    Open { target: String, args: Option<String> },
    /// A PowerShell command.
    Run(String),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    path: String,
    columns: u32,
    buttons: Vec<ButtonView>,
    /// Set when the file could not be read; the page shows it instead.
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ButtonView {
    label: String,
    icon: Option<String>,
    color: Option<String>,
    /// Set when this button's action is invalid, so the mistake shows on its face.
    error: Option<String>,
}

fn path(dir: &Path) -> PathBuf {
    dir.join("macros.json")
}

fn load(dir: &Path) -> Result<File, String> {
    let p = path(dir);
    if !p.exists() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        std::fs::write(&p, STARTER).map_err(|e| e.to_string())?;
    }
    let text = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("macros.json: {e}"))
}

pub fn view(dir: &Path) -> View {
    let (columns, buttons, error) = match load(dir) {
        Ok(f) => (f.columns.clamp(1, 8), f.buttons, None),
        Err(e) => (5, Vec::new(), Some(e)),
    };
    View {
        path: path(dir).display().to_string(),
        columns,
        buttons: buttons
            .into_iter()
            .map(|b| ButtonView {
                error: b.action().err(),
                label: b.label,
                icon: b.icon,
                color: b.color,
            })
            .collect(),
        error,
    }
}

pub fn run(dir: &Path, index: usize, label: &str) -> Result<(), String> {
    let file = load(dir)?;
    let button = file.buttons.get(index).filter(|b| b.label == label);
    // The file can change between the page drawing a button and a tap on it.
    let button = button.ok_or("macros.json changed; showing the new buttons")?;
    execute(button.action()?)
}

impl Button {
    fn action(&self) -> Result<Action, String> {
        let set = [&self.media, &self.keys, &self.open, &self.run]
            .iter()
            .filter(|a| a.is_some())
            .count();
        if set != 1 {
            return Err("needs exactly one of media, keys, open, run".into());
        }
        if let Some(m) = &self.media {
            return media_vk(m).map(|vk| Action::Keys(vec![vk])).ok_or(format!("unknown media key '{m}'"));
        }
        if let Some(k) = &self.keys {
            return parse_keys(k).map(Action::Keys);
        }
        if let Some(o) = self.open.as_deref().filter(|o| !o.trim().is_empty()) {
            return Ok(Action::Open { target: o.into(), args: self.args.clone() });
        }
        if let Some(r) = self.run.as_deref().filter(|r| !r.trim().is_empty()) {
            return Ok(Action::Run(r.into()));
        }
        Err("its action is empty".into())
    }
}

fn media_vk(name: &str) -> Option<u16> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "play_pause" => 0xB3,
        "next" => 0xB0,
        "previous" => 0xB1,
        "stop" => 0xB2,
        "volume_up" => 0xAF,
        "volume_down" => 0xAE,
        "mute" => 0xAD,
        _ => return None,
    })
}

fn key_vk(name: &str) -> Option<u16> {
    let n = name.trim().to_ascii_lowercase();
    if let [c] = n.as_bytes() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            return Some(c.to_ascii_uppercase() as u16);
        }
    }
    if let Some(f) = n.strip_prefix('f').and_then(|d| d.parse::<u16>().ok()) {
        return (1..=24).contains(&f).then_some(0x6F + f);
    }
    Some(match n.as_str() {
        "ctrl" | "control" => 0x11,
        "shift" => 0x10,
        "alt" => 0x12,
        "win" | "windows" | "super" | "meta" => 0x5B,
        "enter" | "return" => 0x0D,
        "esc" | "escape" => 0x1B,
        "tab" => 0x09,
        "space" => 0x20,
        "backspace" => 0x08,
        "delete" | "del" => 0x2E,
        "insert" | "ins" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdn" => 0x22,
        "up" => 0x26,
        "down" => 0x28,
        "left" => 0x25,
        "right" => 0x27,
        "printscreen" | "prtsc" => 0x2C,
        "plus" => 0xBB,
        "minus" => 0xBD,
        _ => return None,
    })
}

/// "ctrl+shift+m" -> the keys to press, in order.
fn parse_keys(combo: &str) -> Result<Vec<u16>, String> {
    combo
        .split('+')
        .map(|k| key_vk(k).ok_or(format!("unknown key '{}' in '{combo}'", k.trim())))
        .collect()
}

#[cfg(windows)]
fn execute(action: Action) -> Result<(), String> {
    use windows::core::{w, HSTRING};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    match action {
        Action::Keys(vks) => {
            let key = |vk: u16, up: bool| {
                let mut flags = if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) };
                // navigation keys sit on the extended keypad; without this
                // flag some apps read them as numpad keys
                if matches!(vk, 0x21..=0x28 | 0x2D | 0x2E | 0x5B) {
                    flags |= KEYEVENTF_EXTENDEDKEY;
                }
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), dwFlags: flags, ..Default::default() },
                    },
                }
            };
            let inputs: Vec<INPUT> = vks
                .iter()
                .map(|&vk| key(vk, false))
                .chain(vks.iter().rev().map(|&vk| key(vk, true)))
                .collect();
            let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
            if sent as usize != inputs.len() {
                // UIPI: a non-elevated app cannot type into an elevated one.
                return Err("Windows blocked the keystrokes".into());
            }
            Ok(())
        }
        Action::Open { target, args } => {
            // ShellExecute may hand off to shell extensions, which want COM on
            // the calling thread; a short-lived thread keeps that off Tauri's pool.
            std::thread::spawn(move || unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
                let args = args.map(HSTRING::from).unwrap_or_default();
                let r = ShellExecuteW(None, w!("open"), &HSTRING::from(&target), &args, None, SW_SHOWNORMAL);
                // values above 32 mean success
                if r.0 as isize > 32 { Ok(()) } else { Err(format!("Windows could not open {target}")) }
            })
            .join()
            .map_err(|_| "opening crashed".to_string())?
        }
        Action::Run(command) => {
            use base64::Engine;
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            // -EncodedCommand takes UTF-16LE base64, which survives any quotes
            // in the command; Windows PowerShell mangles them on a plain -Command.
            let utf16: Vec<u8> = command.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
            std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-EncodedCommand", &encoded])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("could not start PowerShell: {e}"))
        }
    }
}

#[cfg(not(windows))]
fn execute(_: Action) -> Result<(), String> {
    Err("Macros only run on Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_file_is_valid() {
        let file: File = serde_json::from_str(STARTER).expect("starter parses");
        for b in &file.buttons {
            assert!(b.action().is_ok(), "{}: {:?}", b.label, b.action().err());
        }
    }

    #[test]
    fn parses_key_combos() {
        assert_eq!(parse_keys("ctrl+shift+m").unwrap(), vec![0x11, 0x10, 0x4D]);
        assert_eq!(parse_keys("Win + F13").unwrap(), vec![0x5B, 0x7C]);
        assert!(parse_keys("ctl+m").unwrap_err().contains("'ctl'"));
        assert!(parse_keys("f25").is_err());
    }

    #[test]
    fn a_button_needs_exactly_one_action() {
        let none = Button { label: "x".into(), ..Default::default() };
        assert!(none.action().is_err());
        let two = Button { media: Some("mute".into()), run: Some("dir".into()), ..Default::default() };
        assert!(two.action().is_err());
        let typo = Button { media: Some("play".into()), ..Default::default() };
        assert!(typo.action().unwrap_err().contains("'play'"));
    }

    #[test]
    fn writes_the_starter_file_and_guards_against_edits() {
        let dir = std::env::temp_dir().join(format!("macros-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let v = view(&dir);
        assert!(v.error.is_none() && v.buttons.len() == 15);
        // a stale label must not run whatever now sits at that position
        assert!(run(&dir, 0, "Not the label").unwrap_err().contains("changed"));
        std::fs::write(path(&dir), "{ not json").unwrap();
        assert!(view(&dir).error.unwrap().starts_with("macros.json"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
