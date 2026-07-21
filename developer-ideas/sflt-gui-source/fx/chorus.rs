use std::fmt::Display;

use crate::fx::FxOutput;


#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum ChorusMode {
	#[default]
	Disabled = 0,
	Dry = 1,
	Enabled = 2,
}

impl ChorusMode {
	pub fn from_raw(val: i32) -> Option<ChorusMode> {
		match val {
			0 => Some(Self::Disabled),
			1 => Some(Self::Dry),
			2 => Some(Self::Enabled),
			_ => None,
		}
	}
}

impl Display for ChorusMode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			ChorusMode::Disabled => write!(f, "disabled"),
			ChorusMode::Dry => write!(f, "dry"),
			ChorusMode::Enabled => write!(f, "enabled"),
		}
	}
}


#[derive(Default, Debug, Clone)]
pub struct ChorusUnit {
	pub input: [Vec<f32>; 2],
	mode: ChorusMode,
	sample_rate: u32,
}

impl ChorusUnit {
	pub const fn new() -> Self {
		ChorusUnit {
			input: [vec![], vec![]],
			mode: ChorusMode::Disabled,
			sample_rate: 44100,
		}
	}

	pub fn set_mode(&mut self, mode: ChorusMode, sample_rate: u32) {
		self.mode = mode;
		self.sample_rate = sample_rate;
	}

	pub fn process(&mut self, mut output: impl FxOutput, gain: [f32; 2]) {
		self.input[0].resize(output.channel_len(), 0.0);
		self.input[1].resize(output.channel_len(), 0.0);

		match self.mode {
			ChorusMode::Disabled => {},

			ChorusMode::Dry => {
				let [bufl, bufr] = &mut self.input;

				output.write(&[bufl, bufr], gain)
			}

			_ => {}
		}
	}
}