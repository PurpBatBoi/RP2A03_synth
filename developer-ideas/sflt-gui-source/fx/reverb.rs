use std::fmt::Display;

use crate::fx::{Delay, FxOutput};


#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum ReverbMode {
	#[default]
	Disabled = 0,
	Dry = 1,
	Enabled = 2,
}


impl ReverbMode {
	pub fn from_raw(val: i32) -> Option<ReverbMode> {
		match val {
			0 => Some(Self::Disabled),
			1 => Some(Self::Dry),
			2 => Some(Self::Enabled),
			_ => None,
		}
	}
}


impl Display for ReverbMode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			ReverbMode::Disabled => write!(f, "disabled"),
			ReverbMode::Dry => write!(f, "dry"),
			ReverbMode::Enabled => write!(f, "enabled"),
		}
	}
}


const REVERB_CHANNELS: usize = 8;


#[derive(Default, Debug, Clone)]
pub struct ReverbUnit {
	pub input: [Vec<f32>; 2],
	mode: ReverbMode,
	sample_rate: u32,

	delay_ms: f64,
	decay_gain: f32,

	delay_samples: [usize; REVERB_CHANNELS],
	delay: [Delay; REVERB_CHANNELS],
}


impl ReverbUnit {
	fn delay_samples_base(&self) -> usize {
		(self.delay_ms * self.sample_rate as f64 * 0.001) as usize + 1
	}


	pub const fn new() -> Self {
		ReverbUnit {
			input: [vec![], vec![]],
			mode: ReverbMode::Disabled,
			sample_rate: 44100,

			delay_ms: 80.0,
			decay_gain: 0.85,

			delay_samples: [0; REVERB_CHANNELS],
			delay: [const { Delay::new() }; REVERB_CHANNELS],
		}
	}


	pub fn set_mode(&mut self, mode: ReverbMode, sample_rate: u32) {
		self.mode = mode;

		if sample_rate != self.sample_rate {
			self.sample_rate = sample_rate;

			let samples = self.delay_samples_base();

			for i in 0..REVERB_CHANNELS {
				// Distribute delay times exponentially between delay_ms and 2*delay_ms
				let r = i as f64 / REVERB_CHANNELS as f64;

				self.delay_samples[i] = (2f64.powf(r) * samples as f64) as usize;

				self.delay[i].buf.clear();
				self.delay[i].buf.resize(self.delay_samples[i] + 1, 0.0);
			}
		}
	}


	pub fn process(&mut self, mut output: impl FxOutput, gain: [f32; 2]) {
		self.input[0].resize(output.channel_len(), 0.0);
		self.input[1].resize(output.channel_len(), 0.0);

		match self.mode {
			ReverbMode::Disabled => {},

			ReverbMode::Dry => {
				let [bufl, bufr] = &mut self.input;

				output.write(&[bufl, bufr], gain)
			}

			ReverbMode::Enabled => {
				self.delay[0].buf.resize(1000, 0.0);

				let [bufl, bufr] = &mut self.input;

				for s in bufl.iter_mut() {
					self.delay[0].write(*s * self.decay_gain);
					*s = self.delay[0][500];
				}

				output.write(&[bufl, bufr], gain)
			}
		}
	}
}
