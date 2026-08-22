//! `rp2a03_common\src\midi\fds_bridge.rs`
//! FDS wave upload, mod-table upload and the wavesynth tick — the bridge
//! between `ActiveSequences`' authored FDS state and the live `Fds` chip.

use super::handler::MidiHandler;
use super::types::{ActiveSequences, ChannelMode, Lane};
use crate::gui::{
    FDS_MOD_DEPTH_MAX, FDS_MOD_SPEED_MAX, FDS_MOD_TABLE_LEN, FDS_MOD_TABLE_MAX, FDS_MOD_TABLE_MIN,
    TickState, WaveSynthEffect, wavesynth_tick,
};
use rp2a03_core::fds_audio::Fds;

fn widen_fds_wave(src: &[u8; crate::FDS_WAVE_LEN]) -> [u16; crate::FDS_WAVE_LEN] {
    let mut out = [0u16; crate::FDS_WAVE_LEN];
    for (dst, &value) in out.iter_mut().zip(src.iter()) {
        *dst = u16::from(value);
    }
    out
}

#[derive(Debug, Clone)]
pub(super) struct FdsWaveSynth {
    tick_state: TickState,

    output: [u16; crate::FDS_WAVE_LEN],

    seeded: Option<([u16; crate::FDS_WAVE_LEN], bool, WaveSynthEffect)>,
}

impl Default for FdsWaveSynth {
    fn default() -> Self {
        Self {
            tick_state: TickState::default(),
            output: [0; crate::FDS_WAVE_LEN],
            seeded: None,
        }
    }
}

impl FdsWaveSynth {
    pub(super) fn restart(&mut self) {
        self.tick_state = TickState::default();
        self.seeded = None;
    }
}

impl MidiHandler {
    pub(super) fn restart_fds_wavesynth(&mut self) {
        self.fds_wavesynth.restart();
    }

    pub(super) fn arm_fds_mod_delay(&mut self, seqs: &ActiveSequences) {
        self.fds_mod_delay = seqs.fds_settings.mod_delay.clamp(0, 255) as u8;
    }

    fn fds_source_indices(&self, seqs: &ActiveSequences) -> (usize, usize) {
        // Both call sites already guard `fds_waves.is_none()` before using
        // these as indices; this is a second, independent guard so the
        // underflow can never surface even if a future caller forgets to.
        let Some(last) = seqs
            .fds_waves
            .as_deref()
            .and_then(|waves| waves.len().checked_sub(1))
        else {
            return (0, 0);
        };
        let params = &seqs.wavesynth;
        let wave1 = if seqs.lane_active(Lane::Duty) {
            self.seq_players[Lane::Duty].value().max(0) as usize
        } else if params.enabled {
            params.wave1
        } else {
            seqs.fds_current_wave
        };
        (wave1.min(last), params.wave2.min(last))
    }

    fn seed_fds_wavesynth(&mut self, seqs: &ActiveSequences) {
        let Some(waves) = seqs.fds_waves.as_deref() else {
            return;
        };
        let params = &seqs.wavesynth;
        let (index, _) = self.fds_source_indices(seqs);
        let wave1 = widen_fds_wave(&waves[index]);

        let seed = (wave1, params.enabled, params.effect);
        if self.fds_wavesynth.seeded != Some(seed) {
            let forced = self.fds_wavesynth.seeded.is_none();

            let effect_changed = self
                .fds_wavesynth
                .seeded
                .is_some_and(|(_, _, prev)| prev != params.effect);
            if forced || effect_changed || !params.enabled || params.effect.accumulates() {
                self.fds_wavesynth.output = wave1;
            }
            if forced || effect_changed {
                self.fds_wavesynth.tick_state = TickState::default();
            }
            self.fds_wavesynth.seeded = Some(seed);
        }
    }

    pub(super) fn tick_fds_wavesynth(&mut self, seqs: &ActiveSequences) {
        if self.channel_mode != ChannelMode::Fds {
            return;
        }
        let Some(waves) = seqs.fds_waves.as_deref() else {
            return;
        };
        let (index, wave2_index) = self.fds_source_indices(seqs);
        let wave1 = widen_fds_wave(&waves[index]);
        let wave2 = widen_fds_wave(&waves[wave2_index]);
        wavesynth_tick(
            &seqs.wavesynth,
            &mut self.fds_wavesynth.tick_state,
            &mut self.fds_wavesynth.output,
            &wave1,
            &wave2,
            crate::FDS_WAVE_MAX,
        );
    }

    pub(super) fn apply_fds_chip_settings(&self, fds: &mut Fds, seqs: &ActiveSequences) {
        let settings = &seqs.fds_settings;
        if self.fds_mod_delay > 0 {
            fds.disable_modulator();
            return;
        }
        fds.set_mod_speed(settings.mod_speed.clamp(0, FDS_MOD_SPEED_MAX) as u16);
        fds.set_mod_depth(settings.mod_depth.clamp(0, FDS_MOD_DEPTH_MAX) as u8);
    }

    pub(super) fn upload_fds_mod_table(&mut self, fds: &mut Fds, seqs: &ActiveSequences) {
        let authored = &seqs.fds_settings.mod_table.values;
        let mut table = [0i8; FDS_MOD_TABLE_LEN];
        for (out, &value) in table.iter_mut().zip(authored) {
            *out = value.clamp(FDS_MOD_TABLE_MIN, FDS_MOD_TABLE_MAX) as i8;
        }
        if self.uploaded_fds_mod_table == Some(table) {
            return;
        }
        fds.load_mod_table(&table);
        self.uploaded_fds_mod_table = Some(table);
    }

    pub(super) fn upload_fds_wave(&mut self, fds: &mut Fds, seqs: &ActiveSequences) {
        if seqs.fds_waves.is_none() {
            return;
        }

        self.seed_fds_wavesynth(seqs);

        let mut wave = [0u8; crate::FDS_WAVE_LEN];
        for (out, value) in wave.iter_mut().zip(self.fds_wavesynth.output) {
            *out = value as u8;
        }
        if self.uploaded_fds_wave == Some(wave) {
            return;
        }

        fds.load_wave(&wave);
        self.uploaded_fds_wave = Some(wave);
    }
}
