//! `rp2a03_common\src\gui\wavetable_editor\tools.rs`
//! The generator sidebar's "`WaveTools`" tab: transforms applied to the
//! currently selected wavetable slot (offset, smooth, amplify, normalize,
//! invert, reverse, half/double the length, sign conversion, randomize).

use crate::gui::wavetable_state::{FDS_WAVE_LEN, FDS_WAVE_MAX, WaveSlots, WavetableEditorState};

fn offset_x(data: &mut [u16], amount: i32) {
    let len = data.len();
    if len == 0 || amount == 0 {
        return;
    }

    let orig = data.to_vec();
    for (i, v) in data.iter_mut().enumerate() {
        *v = orig[(i as isize - amount as isize).rem_euclid(len as isize) as usize];
    }
}

fn offset_y(data: &mut [u16], max: u16, amount: i32) {
    for v in data.iter_mut() {
        *v = (i32::from(*v) + amount).clamp(0, i32::from(max)) as u16;
    }
}

fn smooth(data: &mut [u16], window: usize) {
    let len = data.len();
    if len == 0 || window == 0 {
        return;
    }

    let orig = data.to_vec();
    let span = window + 1;
    let half = (span / 2) as isize;

    for (i, v) in data.iter_mut().enumerate() {
        let sum: u32 = (0..span)
            .map(|j| {
                let pos = (i + j) as isize - half;
                u32::from(orig[pos.rem_euclid(len as isize) as usize])
            })
            .sum();
        *v = (sum / span as u32) as u16;
    }
}

fn amplify(data: &mut [u16], max: u16, factor: f32) {
    let midpoint = (i32::from(max) + 1) / 2;
    let (lo, hi) = (-midpoint, i32::from(max) / 2);

    for v in data.iter_mut() {
        let centered = (i32::from(*v) - midpoint) as f32 * factor;
        let scaled = (centered.round() as i32).clamp(lo, hi);
        *v = (scaled + midpoint).clamp(0, i32::from(max)) as u16;
    }
}

fn normalize(data: &mut [u16], max: u16) {
    let Some(&lowest) = data.iter().min() else {
        return;
    };
    let highest = *data.iter().max().unwrap();
    if lowest == highest {
        return;
    }

    let span = u32::from(highest - lowest);
    for v in data.iter_mut() {
        *v = ((u32::from(*v - lowest) * u32::from(max)) / span) as u16;
    }
}

fn invert(data: &mut [u16], max: u16) {
    for v in data.iter_mut() {
        *v = max.saturating_sub(*v);
    }
}

fn reverse(data: &mut [u16]) {
    data.reverse();
}

fn half(data: &mut [u16]) {
    let orig = data.to_vec();
    for (i, v) in data.iter_mut().enumerate() {
        *v = orig[i >> 1];
    }
}

fn double(data: &mut [u16]) {
    let len = data.len();
    if len == 0 {
        return;
    }

    let orig = data.to_vec();
    for (i, v) in data.iter_mut().enumerate() {
        *v = orig[(i * 2) % len];
    }
}

fn convert_sign(data: &mut [u16], max: u16) {
    if max == 0 {
        return;
    }

    let half_span = max.div_ceil(2);
    for v in data.iter_mut() {
        *v = if *v > max / 2 {
            *v - half_span
        } else {
            *v + half_span
        };
    }
}

fn randomize(data: &mut [u16], max: u16, rng: &mut Xorshift) {
    if max == 0 {
        return;
    }

    for v in data.iter_mut() {
        *v = (rng.next() % (u32::from(max) + 1)) as u16;
    }
}

struct Xorshift(u32);

impl Xorshift {
    fn from_time() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x1234_5678, |d| d.subsec_nanos());

        Self(nanos | 1)
    }

    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
}

pub(super) fn draw_wavetools_tab(
    ui: &mut egui::Ui,
    state: &mut WavetableEditorState,
    slots: &mut WaveSlots,
) {
    if slots.current().is_none() {
        return;
    }
    let (width, max) = (FDS_WAVE_LEN, FDS_WAVE_MAX);

    ui.add_space(4.0);

    ui.horizontal(|ui| {
        let span = width.saturating_sub(1) as i32;
        ui.add(egui::DragValue::new(&mut state.tools.offset_x).range(-span..=span));
        if ui.button("Offset X").clicked() {
            let amount = state.tools.offset_x;
            offset_x(slots.current_mut().unwrap().data_mut(), amount);
        }
    });

    ui.horizontal(|ui| {
        let span = i32::from(max);
        ui.add(egui::DragValue::new(&mut state.tools.offset_y).range(-span..=span));
        if ui.button("Offset Y").clicked() {
            let amount = state.tools.offset_y;
            offset_y(slots.current_mut().unwrap().data_mut(), max, amount);
        }
    });

    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut state.tools.smooth).range(1..=width.max(1)));
        if ui.button("Smooth").clicked() {
            let window = state.tools.smooth;
            smooth(slots.current_mut().unwrap().data_mut(), window);
        }
    });

    ui.horizontal(|ui| {
        let mut percent = state.tools.amplify * 100.0;
        if ui
            .add(
                egui::DragValue::new(&mut percent)
                    .range(0.0..=1000.0)
                    .suffix("%"),
            )
            .changed()
        {
            state.tools.amplify = percent / 100.0;
        }
        if ui.button("Amplify").clicked() {
            let factor = state.tools.amplify;
            amplify(slots.current_mut().unwrap().data_mut(), max, factor);
        }
    });

    ui.add_space(6.0);
    ui.separator();
    ui.add_space(4.0);

    if ui.button("Normalize").clicked() {
        normalize(slots.current_mut().unwrap().data_mut(), max);
    }

    ui.horizontal(|ui| {
        if ui.button("Invert").clicked() {
            invert(slots.current_mut().unwrap().data_mut(), max);
        }
        if ui.button("Reverse").clicked() {
            reverse(slots.current_mut().unwrap().data_mut());
        }
    });

    ui.horizontal(|ui| {
        if ui.button("Half").clicked() {
            half(slots.current_mut().unwrap().data_mut());
        }
        if ui.button("Double").clicked() {
            double(slots.current_mut().unwrap().data_mut());
        }
    });

    if ui.button("Convert Signed/Unsigned").clicked() {
        convert_sign(slots.current_mut().unwrap().data_mut(), max);
    }

    if ui.button("Randomize").clicked() {
        let mut rng = Xorshift::from_time();
        randomize(slots.current_mut().unwrap().data_mut(), max, &mut rng);
    }
}
