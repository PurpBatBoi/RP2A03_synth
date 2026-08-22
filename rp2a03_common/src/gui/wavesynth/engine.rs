//! `rp2a03_common\src\gui\wavesynth\engine.rs`
//! The wave-synthesizer's pure tick algorithm — no egui in it. Reused
//! outside the GUI thread's preview: `midi/fds_bridge.rs` runs the same
//! `tick` to compute what actually gets uploaded to the FDS chip.

use super::{FDS_WAVE_LEN, FDS_WAVE_MAX};
use crate::gui::wavesynth_state::{TickState, WaveSynthEffect, WaveSynthParams};

#[must_use]
pub fn fds_wave_from_slot(src: &[u16]) -> [u8; FDS_WAVE_LEN] {
    let mut wave = [0u8; FDS_WAVE_LEN];
    for (out, &value) in wave.iter_mut().zip(src) {
        *out = value.min(FDS_WAVE_MAX) as u8;
    }
    wave
}

pub fn tick(
    params: &WaveSynthParams,
    tick_state: &mut TickState,
    output: &mut [u16],
    wave1: &[u16],
    wave2: &[u16],
    height: u16,
) -> bool {
    let width = output.len();

    if !params.enabled || width < 1 || wave1.len() != width || wave2.len() != width {
        return false;
    }

    tick_state.div_counter -= 1;
    if tick_state.div_counter > 0 {
        return false;
    }
    tick_state.div_counter = i32::from(params.rate_divider);

    let h = i32::from(height);
    let param1 = i32::from(params.param1);
    let param2 = i32::from(params.param2);
    let mut updated = false;

    for _ in 0..=params.speed {
        let pos = tick_state.pos;
        let old = i32::from(output[pos]);
        let stage = tick_state.stage;

        let new = match params.effect {
            WaveSynthEffect::None => i32::from(wave1[pos]),

            WaveSynthEffect::Invert => h - old,

            WaveSynthEffect::Add => {
                let v = old + param1.min(h);
                if v >= h { v - h } else { v }
            }
            WaveSynthEffect::Subtract => {
                let v = old - param1.min(h);
                if v < 0 { v + h } else { v }
            }
            WaveSynthEffect::Average => {
                let next = i32::from(output[(pos + 1) % width]);

                let v = (128 + old * (256 - param1) + next * param1) >> 8;
                v.clamp(0, h)
            }
            WaveSynthEffect::Phase => i32::from(wave1[(pos + stage as usize) % width]),
            WaveSynthEffect::Chorus => {
                (i32::from(wave1[pos]) + i32::from(wave1[(pos + stage as usize) % width])) >> 1
            }

            WaveSynthEffect::Wipe => {
                let v = i32::from(if stage & 1 != 0 {
                    wave1[pos]
                } else {
                    wave2[pos]
                });
                v.clamp(0, h)
            }
            WaveSynthEffect::Fade => {
                let (a, b) = (i32::from(wave1[pos]), i32::from(wave2[pos]));
                a + (((b - a) * stage) >> 9)
            }
            WaveSynthEffect::PingPong => {
                let (a, b) = (i32::from(wave1[pos]), i32::from(wave2[pos]));
                a + (((b - a) * stage) >> 8)
            }
            WaveSynthEffect::Overlay => {
                let v = old + i32::from(wave2[pos]);
                if v >= h { v - h } else { v }
            }
            WaveSynthEffect::NegativeOverlay => {
                let v = old - i32::from(wave2[pos]);
                if v < 0 { v + h } else { v }
            }
            WaveSynthEffect::Slide => {
                let new_pos = (pos + stage as usize) % (width * 2);
                if new_pos >= width {
                    i32::from(wave2[new_pos - width])
                } else {
                    i32::from(wave1[new_pos])
                }
            }
            WaveSynthEffect::Mix => {
                (i32::from(wave1[pos]) + i32::from(wave2[(pos + stage as usize) % width])) >> 1
            }
            WaveSynthEffect::PhaseModulation => {
                let modulation =
                    (i32::from(wave2[pos]) * (param2 - stage) * width as i32) / (64 * (h + 1));
                let idx = (pos as i32 + modulation).rem_euclid(width as i32) as usize;
                i32::from(wave1[idx])
            }
        };

        let clamped = new.clamp(0, h) as u16;
        if clamped != old as u16 {
            output[pos] = clamped;
            updated = true;
        }

        tick_state.pos += 1;
        if tick_state.pos >= width {
            tick_state.pos = 0;
            advance_stage(params, tick_state, width);
        }
    }

    updated
}

fn advance_stage(params: &WaveSynthParams, ts: &mut TickState, width: usize) {
    let param1 = i32::from(params.param1);
    let param2 = i32::from(params.param2);
    let w = width as i32;

    match params.effect {
        WaveSynthEffect::Phase => {
            ts.stage += 1;
            if ts.stage >= w {
                ts.stage = 0;
            }
        }
        WaveSynthEffect::Chorus | WaveSynthEffect::Mix => {
            ts.stage += param1;
            while ts.stage >= w {
                ts.stage -= w;
            }
        }
        WaveSynthEffect::Wipe => ts.stage = i32::from(ts.stage == 0),
        WaveSynthEffect::Fade => {
            ts.stage += 1 + param1;
            if ts.stage > 512 {
                ts.stage = 512;
            }
        }
        WaveSynthEffect::PingPong => {
            if ts.stage_dir {
                ts.stage -= 1 + param1;
                if ts.stage <= 0 {
                    ts.stage_dir = false;
                    ts.stage = 0;
                }
            } else {
                ts.stage += 1 + param1;
                if ts.stage >= 256 {
                    ts.stage_dir = true;
                    ts.stage = 256;
                }
            }
        }
        WaveSynthEffect::Slide => {
            ts.stage += 1;
            if ts.stage >= w * 2 {
                ts.stage = 0;
            }
        }
        WaveSynthEffect::PhaseModulation => {
            ts.stage += param1;
            if ts.stage > param2 {
                ts.stage = param2;
            }
        }

        _ => {}
    }
}
