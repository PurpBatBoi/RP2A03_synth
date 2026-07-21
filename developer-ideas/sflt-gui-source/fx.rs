use std::ops::{Index, IndexMut};

mod chorus;
mod reverb;

pub use chorus::{ChorusMode, ChorusUnit};
pub use reverb::{ReverbMode, ReverbUnit};

pub trait FxOutput {
	fn write(&mut self, output: &[&[f32]], gain: [f32; 2]);
	fn channel_len(&self) -> usize;
}

impl FxOutput for &mut [[f32; 2]] {
	fn write(&mut self, output: &[&[f32]], gain: [f32; 2]) {
		for i in 0..output.len() {
			for j in 0..output[i].len() {
				self[j][i] += output[i][j] * gain[i];
			}
		}
	}

	fn channel_len(&self) -> usize {
		self.len()
	}
}

impl FxOutput for &mut [&mut [f32]] {
	fn write(&mut self, output: &[&[f32]], gain: [f32; 2]) {
		for (i, chan) in self.iter_mut().zip(output).enumerate() {
			for (j, s) in chan.0.iter_mut().enumerate() {
				*s += output[i][j] * gain[i];
			}
		}
	}

	fn channel_len(&self) -> usize {
		self[0].len()
	}
}

#[derive(Default, Debug, Clone)]
struct Delay {
	buf: Vec<f32>,
	pos: usize,
}

impl Delay {
	pub const fn new() -> Self {
		Delay {
			buf: vec![],
			pos: 0
		}
	}

	fn write(&mut self, value: f32) {
		self.pos += 1;
		self[0] = value;
	}
}

impl Index<usize> for Delay {
	type Output = f32;

	fn index(&self, index: usize) -> &Self::Output {
		let len = self.buf.len();
		unsafe { self.buf.get_unchecked((self.pos + index) % len) }
	}
}

impl IndexMut<usize> for Delay {
	fn index_mut(&mut self, index: usize) -> &mut Self::Output {
		let len = self.buf.len();
		unsafe { self.buf.get_unchecked_mut((self.pos + index) % len) }
	}
}