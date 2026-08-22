//! `rp2a03_common\src\gui\chip_settings_state.rs`

use rp2a03_core::sequencer::Sequence;

pub const FDS_MOD_TABLE_LEN: usize = 32;

pub const FDS_MOD_TABLE_MIN: i16 = -4;
pub const FDS_MOD_TABLE_MAX: i16 = 3;

pub const FDS_MOD_DEPTH_MAX: i32 = 63;
pub const FDS_MOD_SPEED_MAX: i32 = 4095;

pub const FDS_MOD_DELAY_MAX: i32 = 255;

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FdsSettings {
    pub mod_depth: i32,

    pub mod_speed: i32,

    pub mod_table: Sequence,

    #[serde(default)]
    pub mod_delay: i32,
}

impl Default for FdsSettings {
    fn default() -> Self {
        Self {
            mod_depth: 0,
            mod_speed: 0,
            mod_table: Sequence {
                values: vec![0; FDS_MOD_TABLE_LEN],
                ..Default::default()
            },
            mod_delay: 0,
        }
    }
}

impl Clone for FdsSettings {
    fn clone(&self) -> Self {
        Self {
            mod_depth: self.mod_depth,
            mod_speed: self.mod_speed,
            mod_table: self.mod_table.clone(),
            mod_delay: self.mod_delay,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.mod_depth = source.mod_depth;
        self.mod_speed = source.mod_speed;
        self.mod_table.clone_from(&source.mod_table);
        self.mod_delay = source.mod_delay;
    }
}

pub const N163_MAX_CHANNELS: i32 = 8;

pub struct N163Settings {
    pub channels: i32,

    pub high_quality_mixer: bool,
}

impl Default for N163Settings {
    fn default() -> Self {
        Self {
            channels: 1,
            high_quality_mixer: false,
        }
    }
}

pub fn encode_mod_table_text(values: &[i16]) -> String {
    values
        .iter()
        .take(FDS_MOD_TABLE_LEN)
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn decode_mod_table_text(text: &str) -> Vec<i16> {
    let mut values: Vec<i16> = text
        .replace(',', " ")
        .split_whitespace()
        .filter_map(|token| {
            if matches!(token, "|" | "/" | "L" | "l" | "R" | "r") {
                return None;
            }
            token
                .parse::<i16>()
                .ok()
                .map(|v| v.clamp(FDS_MOD_TABLE_MIN, FDS_MOD_TABLE_MAX))
        })
        .take(FDS_MOD_TABLE_LEN)
        .collect();
    values.resize(FDS_MOD_TABLE_LEN, 0);
    values
}

#[derive(Default)]
pub struct ChipSettingsState {
    pub n163: N163Settings,

    pub mod_table_text: String,
}
