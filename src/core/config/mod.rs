use std::collections::HashMap;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use tracing::error;
use winit::keyboard::KeyCode;
use crate::core::controllers::ControllerButton;

pub fn parse_keycode(s: &str) -> Option<KeyCode> {
    match s {
        // Lettere
        "KeyA" | "A" => Some(KeyCode::KeyA),
        "KeyB" | "B" => Some(KeyCode::KeyB),
        "KeyC" | "C" => Some(KeyCode::KeyC),
        "KeyD" | "D" => Some(KeyCode::KeyD),
        "KeyE" | "E" => Some(KeyCode::KeyE),
        "KeyF" | "F" => Some(KeyCode::KeyF),
        "KeyG" | "G" => Some(KeyCode::KeyG),
        "KeyH" | "H" => Some(KeyCode::KeyH),
        "KeyI" | "I" => Some(KeyCode::KeyI),
        "KeyJ" | "J" => Some(KeyCode::KeyJ),
        "KeyK" | "K" => Some(KeyCode::KeyK),
        "KeyL" | "L" => Some(KeyCode::KeyL),
        "KeyM" | "M" => Some(KeyCode::KeyM),
        "KeyN" | "N" => Some(KeyCode::KeyN),
        "KeyO" | "O" => Some(KeyCode::KeyO),
        "KeyP" | "P" => Some(KeyCode::KeyP),
        "KeyQ" | "Q" => Some(KeyCode::KeyQ),
        "KeyR" | "R" => Some(KeyCode::KeyR),
        "KeyS" | "S" => Some(KeyCode::KeyS),
        "KeyT" | "T" => Some(KeyCode::KeyT),
        "KeyU" | "U" => Some(KeyCode::KeyU),
        "KeyV" | "V" => Some(KeyCode::KeyV),
        "KeyW" | "W" => Some(KeyCode::KeyW),
        "KeyX" | "X" => Some(KeyCode::KeyX),
        "KeyY" | "Y" => Some(KeyCode::KeyY),
        "KeyZ" | "Z" => Some(KeyCode::KeyZ),

        // Numeri
        "Digit0" | "0" => Some(KeyCode::Digit0),
        "Digit1" | "1" => Some(KeyCode::Digit1),
        "Digit2" | "2" => Some(KeyCode::Digit2),
        "Digit3" | "3" => Some(KeyCode::Digit3),
        "Digit4" | "4" => Some(KeyCode::Digit4),
        "Digit5" | "5" => Some(KeyCode::Digit5),
        "Digit6" | "6" => Some(KeyCode::Digit6),
        "Digit7" | "7" => Some(KeyCode::Digit7),
        "Digit8" | "8" => Some(KeyCode::Digit8),
        "Digit9" | "9" => Some(KeyCode::Digit9),

        // Frecce
        "ArrowUp" | "Up" => Some(KeyCode::ArrowUp),
        "ArrowDown" | "Down" => Some(KeyCode::ArrowDown),
        "ArrowLeft" | "Left" => Some(KeyCode::ArrowLeft),
        "ArrowRight" | "Right" => Some(KeyCode::ArrowRight),

        // Speciali
        "Enter" | "Return" => Some(KeyCode::Enter),
        "Space" => Some(KeyCode::Space),
        "Escape" | "Esc" => Some(KeyCode::Escape),
        "Tab" => Some(KeyCode::Tab),
        "Backspace" => Some(KeyCode::Backspace),

        // Modificatori
        "ShiftLeft" | "LShift" => Some(KeyCode::ShiftLeft),
        "ShiftRight" | "RShift" => Some(KeyCode::ShiftRight),
        "ControlLeft" | "LCtrl" | "Ctrl" => Some(KeyCode::ControlLeft),
        "ControlRight" | "RCtrl" => Some(KeyCode::ControlRight),
        "AltLeft" | "LAlt" | "Alt" => Some(KeyCode::AltLeft),
        "AltRight" | "RAlt" => Some(KeyCode::AltRight),

        // F-keys
        "F1" => Some(KeyCode::F1),
        "F2" => Some(KeyCode::F2),
        "F3" => Some(KeyCode::F3),
        "F4" => Some(KeyCode::F4),
        "F5" => Some(KeyCode::F5),
        "F6" => Some(KeyCode::F6),
        "F7" => Some(KeyCode::F7),
        "F8" => Some(KeyCode::F8),
        "F9" => Some(KeyCode::F9),
        "F10" => Some(KeyCode::F10),
        "F11" => Some(KeyCode::F11),
        "F12" => Some(KeyCode::F12),

        _ => None,
    }
}

pub fn keycode_to_string(keycode: KeyCode) -> String {
    match keycode {
        // Lettere
        KeyCode::KeyA => "KeyA".to_string(),
        KeyCode::KeyB => "KeyB".to_string(),
        KeyCode::KeyC => "KeyC".to_string(),
        KeyCode::KeyD => "KeyD".to_string(),
        KeyCode::KeyE => "KeyE".to_string(),
        KeyCode::KeyF => "KeyF".to_string(),
        KeyCode::KeyG => "KeyG".to_string(),
        KeyCode::KeyH => "KeyH".to_string(),
        KeyCode::KeyI => "KeyI".to_string(),
        KeyCode::KeyJ => "KeyJ".to_string(),
        KeyCode::KeyK => "KeyK".to_string(),
        KeyCode::KeyL => "KeyL".to_string(),
        KeyCode::KeyM => "KeyM".to_string(),
        KeyCode::KeyN => "KeyN".to_string(),
        KeyCode::KeyO => "KeyO".to_string(),
        KeyCode::KeyP => "KeyP".to_string(),
        KeyCode::KeyQ => "KeyQ".to_string(),
        KeyCode::KeyR => "KeyR".to_string(),
        KeyCode::KeyS => "KeyS".to_string(),
        KeyCode::KeyT => "KeyT".to_string(),
        KeyCode::KeyU => "KeyU".to_string(),
        KeyCode::KeyV => "KeyV".to_string(),
        KeyCode::KeyW => "KeyW".to_string(),
        KeyCode::KeyX => "KeyX".to_string(),
        KeyCode::KeyY => "KeyY".to_string(),
        KeyCode::KeyZ => "KeyZ".to_string(),

        // Numeri
        KeyCode::Digit0 => "Digit0".to_string(),
        KeyCode::Digit1 => "Digit1".to_string(),
        KeyCode::Digit2 => "Digit2".to_string(),
        KeyCode::Digit3 => "Digit3".to_string(),
        KeyCode::Digit4 => "Digit4".to_string(),
        KeyCode::Digit5 => "Digit5".to_string(),
        KeyCode::Digit6 => "Digit6".to_string(),
        KeyCode::Digit7 => "Digit7".to_string(),
        KeyCode::Digit8 => "Digit8".to_string(),
        KeyCode::Digit9 => "Digit9".to_string(),

        // Frecce
        KeyCode::ArrowUp => "ArrowUp".to_string(),
        KeyCode::ArrowDown => "ArrowDown".to_string(),
        KeyCode::ArrowLeft => "ArrowLeft".to_string(),
        KeyCode::ArrowRight => "ArrowRight".to_string(),

        // Speciali
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Space => "Space".to_string(),
        KeyCode::Escape => "Escape".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),

        // Modificatori
        KeyCode::ShiftLeft => "ShiftLeft".to_string(),
        KeyCode::ShiftRight => "ShiftRight".to_string(),
        KeyCode::ControlLeft => "ControlLeft".to_string(),
        KeyCode::ControlRight => "ControlRight".to_string(),
        KeyCode::AltLeft => "AltLeft".to_string(),
        KeyCode::AltRight => "AltRight".to_string(),

        // F-keys
        KeyCode::F1 => "F1".to_string(),
        KeyCode::F2 => "F2".to_string(),
        KeyCode::F3 => "F3".to_string(),
        KeyCode::F4 => "F4".to_string(),
        KeyCode::F5 => "F5".to_string(),
        KeyCode::F6 => "F6".to_string(),
        KeyCode::F7 => "F7".to_string(),
        KeyCode::F8 => "F8".to_string(),
        KeyCode::F9 => "F9".to_string(),
        KeyCode::F10 => "F10".to_string(),
        KeyCode::F11 => "F11".to_string(),
        KeyCode::F12 => "F12".to_string(),

        _ => format!("{:?}", keycode),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyMapping {
    pub cross: String,
    pub circle: String,
    pub square: String,
    pub triangle: String,
    pub l1: String,
    pub l2: String,
    pub r1: String,
    pub r2: String,
    pub l3: Option<String>,
    pub r3: Option<String>,
    pub start: String,
    pub select: String,
    pub dpad_up: String,
    pub dpad_down: String,
    pub dpad_left: String,
    pub dpad_right: String,
}

impl KeyMapping {
    fn default_controller1() -> Self {
        Self {
            cross: "KeyX".to_string(),
            circle: "KeyZ".to_string(),
            square: "KeyA".to_string(),
            triangle: "KeyS".to_string(),

            // Shoulder buttons
            l1: "KeyQ".to_string(),
            l2: "KeyW".to_string(),
            r1: "KeyE".to_string(),
            r2: "KeyR".to_string(),

            // Analog sticks
            l3: None,
            r3: None,

            // Start/Select
            start: "Enter".to_string(),
            select: "ShiftRight".to_string(),

            // D-Pad
            dpad_up: "ArrowUp".to_string(),
            dpad_down: "ArrowDown".to_string(),
            dpad_left: "ArrowLeft".to_string(),
            dpad_right: "ArrowRight".to_string(),
        }
    }
    fn default_controller2() -> Self {
        Self {
            cross: "KeyK".to_string(),
            circle: "KeyJ".to_string(),
            square: "KeyL".to_string(),
            triangle: "KeyH".to_string(),

            // Shoulder buttons
            l1: "KeyU".to_string(),
            l2: "KeyI".to_string(),
            r1: "KeyO".to_string(),
            r2: "KeyP".to_string(),

            // Analog sticks
            l3: None,
            r3: None,

            // Start/Select
            start: "Digit0".to_string(),
            select: "Digit1".to_string(),

            // D-Pad
            dpad_up: "KeyY".to_string(),
            dpad_down: "KeyH".to_string(),
            dpad_left: "KeyN".to_string(),
            dpad_right: "KeyM".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(into = "KeyMapping")]
pub struct ControllerKeyMapping {
    pub key_mapping: KeyMapping,
    pub key_map: HashMap<KeyCode, ControllerButton>,
}

impl<'de> serde::Deserialize<'de> for ControllerKeyMapping {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mapping = KeyMapping::deserialize(d)?;
        Ok(ControllerKeyMapping::from_config(mapping))
    }
}

impl From<ControllerKeyMapping> for KeyMapping {
    fn from(mapper: ControllerKeyMapping) -> Self {
        mapper.key_mapping
    }
}

impl ControllerKeyMapping {
    pub fn default_controller_1() -> Self {
        Self::from_config(KeyMapping::default_controller1())
    }
    pub fn default_controller_2() -> Self {
        Self::from_config(KeyMapping::default_controller2())
    }

    pub fn map_key(&self, key: KeyCode) -> Option<ControllerButton> {
        self.key_map.get(&key).copied()
    }

    pub fn from_config(config: KeyMapping) -> Self {
        let mut key_map = HashMap::new();

        // Face buttons
        if let Some(key) = parse_keycode(&config.cross) {
            key_map.insert(key, ControllerButton::Cross);
        }
        if let Some(key) = parse_keycode(&config.circle) {
            key_map.insert(key, ControllerButton::Circle);
        }
        if let Some(key) = parse_keycode(&config.square) {
            key_map.insert(key, ControllerButton::Square);
        }
        if let Some(key) = parse_keycode(&config.triangle) {
            key_map.insert(key, ControllerButton::Triangle);
        }

        // Shoulder buttons
        if let Some(key) = parse_keycode(&config.l1) {
            key_map.insert(key, ControllerButton::L1);
        }
        if let Some(key) = parse_keycode(&config.l2) {
            key_map.insert(key, ControllerButton::L2);
        }
        if let Some(key) = parse_keycode(&config.r1) {
            key_map.insert(key, ControllerButton::R1);
        }
        if let Some(key) = parse_keycode(&config.r2) {
            key_map.insert(key, ControllerButton::R2);
        }

        // Analog sticks (opzionale)
        if let Some(ref l3_key) = config.l3 {
            if let Some(key) = parse_keycode(l3_key) {
                key_map.insert(key, ControllerButton::L3);
            }
        }
        if let Some(ref r3_key) = config.r3 {
            if let Some(key) = parse_keycode(r3_key) {
                key_map.insert(key, ControllerButton::R3);
            }
        }

        // Start/Select
        if let Some(key) = parse_keycode(&config.start) {
            key_map.insert(key, ControllerButton::Start);
        }
        if let Some(key) = parse_keycode(&config.select) {
            key_map.insert(key, ControllerButton::Select);
        }

        // D-Pad
        if let Some(key) = parse_keycode(&config.dpad_up) {
            key_map.insert(key, ControllerButton::Up);
        }
        if let Some(key) = parse_keycode(&config.dpad_down) {
            key_map.insert(key, ControllerButton::Down);
        }
        if let Some(key) = parse_keycode(&config.dpad_left) {
            key_map.insert(key, ControllerButton::Left);
        }
        if let Some(key) = parse_keycode(&config.dpad_right) {
            key_map.insert(key, ControllerButton::Right);
        }

        Self {
            key_mapping: config,
            key_map,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize,Default)]
pub enum RegionPolicyConfig {
    #[default]
    Auto,
    Usa,
    Japan,
    Europe
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControllerConfig {
    pub controller_enabled: bool,
    pub controller_keymap: ControllerKeyMapping,
    pub memory_card_path: Option<String>,
    pub attach_to_usb: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControllersConfig {
    pub controller_1: ControllerConfig,
    pub controller_2: ControllerConfig,
    pub save_writings_to_disk: bool,
    pub auto_discover_usb_controllers: bool,
    pub usb_direction_resolution: f32,
    pub tx_rx_cycles: Option<usize>,
}

impl Default for ControllersConfig {
    fn default() -> Self {
        Self {
            controller_1: ControllerConfig {
                controller_enabled: true,
                controller_keymap: ControllerKeyMapping::default_controller_1(),
                memory_card_path: None,
                attach_to_usb: true,
            },
            controller_2: ControllerConfig {
                controller_enabled: true,
                controller_keymap: ControllerKeyMapping::default_controller_2(),
                memory_card_path: None,
                attach_to_usb: true,
            },
            save_writings_to_disk: true,
            auto_discover_usb_controllers: true,
            usb_direction_resolution: 0.1,
            tx_rx_cycles: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub buffer_capacity_in_millis: usize,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            buffer_capacity_in_millis: 10,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryConfig {
    pub cpu_write_queue_enabled: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    pub log_file: Option<PathBuf>,
    pub log_severity: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            log_file: None,
            log_severity: "info".to_string(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GPUConfig {
    pub command_delay_enabled: bool,
    pub rendering_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CdromConfig {
    pub show_cdrom_access: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CheatsConfig {
    pub cheats_enabled: bool,
    pub cheats_codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize,Default)]
pub struct Config {
    #[serde(skip)]
    pub file_config: Option<PathBuf>,
    pub disc_path: Option<String>,
    pub bios_path: Option<String>,
    pub region_policy: RegionPolicyConfig,
    pub controllers: ControllersConfig,
    pub audio_config: AudioConfig,
    pub tty_enabled: bool,
    pub debugger_enabled: bool,
    pub memory_config: MemoryConfig,
    pub log_config: LogConfig,
    pub gpu_config: GPUConfig,
    pub cdrom_config: CdromConfig,
    pub cheats_config: CheatsConfig,
}

impl Config {
    pub fn load_or_default(path: &PathBuf) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| {
                let config = serde_yaml::from_str(&text).ok();
                config.map(|mut config : Config| {
                    config.file_config = Some(path.clone());
                    config
                })
            }).unwrap_or_else(|| {
                error!("Config file not found or an error occurred during deserialization. Using default values.");
                Config::default()
            })
    }

    pub fn save(&self, path: &PathBuf) -> Result<(), String> {
        match serde_yaml::to_string(self) {
            Ok(text) => {
                match std::fs::write(path, text) {
                    Ok(_) => Ok(()),
                    Err(e) => Err(format!("Error writing to file: {}", e)),
                }
            }
            Err(e) => {
                Err(format!("Error serializing configuration to file: {}", e))
            }
        }
    }
}

