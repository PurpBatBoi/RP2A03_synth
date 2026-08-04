//! rp2a03_niceplug\src\sequences.rs
//! The two sequence-state bridges between the editor and the audio thread:
//! `SequenceCache` pulls envelope data down for playback, `PlayheadPublisher`
//! pushes playback positions back up for display.

use parking_lot::Mutex;
use rp2a03_common::{
    ActiveSequences, MidiHandler, NO_PLAYHEAD_STEP, SEQUENCE_TYPE_COUNT, SequencePlayheads,
    SequenceReload, SharedSequences,
};
use rp2a03_core::sequencer::{SeqState, Sequence, SequencePlayer};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Audio-thread-owned copy of the sequences currently being played, plus the
/// key identifying which editor state it was copied from.
#[derive(Default)]
pub(crate) struct SequenceCache {
    pub(crate) active: ActiveSequences,
    /// (sequence_index, shared_sequences revision) as of the last refresh.
    key: Option<(usize, u64)>,
}

impl SequenceCache {
    /// Refreshes `self.active` for `sequence_index` if needed, returning
    /// `Some(reload)` when the *slot* changed and the voices' sequence players
    /// must be re-pointed at the new envelopes.
    ///
    /// Uses `try_lock` and returns `None` when the editor already holds the
    /// mutex: it keeps that lock for the whole egui frame it spends drawing, so
    /// blocking here would stall the audio thread behind a UI repaint. Skipping
    /// costs one block of staleness — the cached envelopes stay valid and the
    /// next `process` retries, because `self.key` is only advanced on a
    /// successful refresh.
    ///
    /// On a successful lock it reads the revision counter first; if
    /// `(sequence_index, revision)` matches the last refresh, it returns
    /// immediately without cloning. On an actual change, `Sequence::clone_from`
    /// reuses each `Vec`'s existing allocation instead of allocating fresh —
    /// this runs on the audio thread, where allocation and long lock hold times
    /// are both real-time-safety hazards.
    ///
    /// A refresh caused by a *slot switch* is dn's `LoadInstrument`; see
    /// `MidiHandler::reload_sequences`. A refresh caused only by a revision bump
    /// is an in-place edit of the slot already playing, which dn does not treat
    /// as a reload, so the players are left alone and `None` is returned.
    pub(crate) fn refresh(
        &mut self,
        shared: &Mutex<SharedSequences>,
        sequence_index: usize,
    ) -> Option<SequenceReload> {
        // Contended: the editor is mid-repaint. Keep the cached envelopes and
        // retry next block rather than block the audio thread.
        let mut data = shared.try_lock()?;
        let revision = data.revision();
        if self.key == Some((sequence_index, revision)) {
            return None;
        }
        data.set_all_selected_sequence_indices(sequence_index);

        let slot_changed = self.key.map(|(index, _)| index) != Some(sequence_index);
        // Compared before the clones below overwrite the outgoing slot's data.
        // Comparing `Vec<i16>` contents allocates nothing, so this stays RT-safe.
        let reload = if slot_changed {
            SequenceReload {
                vol: self.active.vol_seq != *data.selected_sequence(0),
                arp: self.active.arp_seq != *data.selected_sequence(1),
                pitch: self.active.pitch_seq != *data.selected_sequence(2),
                hipitch: self.active.hipitch_seq != *data.selected_sequence(3),
                duty: self.active.duty_seq != *data.selected_sequence(4),
            }
        } else {
            SequenceReload::default()
        };

        self.active.vol_seq.clone_from(data.selected_sequence(0));
        self.active.vol_enabled = data.sequence_enabled(0);
        self.active.arp_seq.clone_from(data.selected_sequence(1));
        self.active.arp_enabled = data.sequence_enabled(1);
        self.active.pitch_seq.clone_from(data.selected_sequence(2));
        self.active.pitch_enabled = data.sequence_enabled(2);
        self.active
            .hipitch_seq
            .clone_from(data.selected_sequence(3));
        self.active.hipitch_enabled = data.sequence_enabled(3);
        self.active.duty_seq.clone_from(data.selected_sequence(4));
        self.active.duty_enabled = data.sequence_enabled(4);
        drop(data);
        self.key = Some((sequence_index, revision));

        // Reported even when every `reload` flag is false: an envelope whose
        // content is identical across the two slots can still have flipped its
        // enable flag, and dn's `ClearSequence` branch is unconditional on the
        // enable state.
        slot_changed.then_some(reload)
    }
}

/// Lock-free channel carrying the audio thread's sequence playhead positions to
/// the editor, one step index per sequence type.
pub(crate) struct PlayheadPublisher {
    steps: Arc<[AtomicUsize; SEQUENCE_TYPE_COUNT]>,
}

impl PlayheadPublisher {
    pub(crate) fn new() -> Self {
        Self {
            steps: Arc::new(std::array::from_fn(|_| AtomicUsize::new(NO_PLAYHEAD_STEP))),
        }
    }

    /// A handle the editor closure can hold and poll with [`snapshot`].
    pub(crate) fn handle(&self) -> Arc<[AtomicUsize; SEQUENCE_TYPE_COUNT]> {
        self.steps.clone()
    }

    pub(crate) fn clear(&self) {
        for step in self.steps.iter() {
            step.store(NO_PLAYHEAD_STEP, Ordering::Relaxed);
        }
    }

    pub(crate) fn publish(&self, handler: &MidiHandler, seqs: &ActiveSequences) {
        let positions = [
            play_step(&handler.vol_seq_player, &seqs.vol_seq, seqs.vol_enabled),
            play_step(&handler.arp_seq_player, &seqs.arp_seq, seqs.arp_enabled),
            play_step(
                &handler.pitch_seq_player,
                &seqs.pitch_seq,
                seqs.pitch_enabled,
            ),
            play_step(
                &handler.hipitch_seq_player,
                &seqs.hipitch_seq,
                seqs.hipitch_enabled,
            ),
            play_step(&handler.duty_seq_player, &seqs.duty_seq, seqs.duty_enabled),
        ];

        for (step, position) in self.steps.iter().zip(positions) {
            step.store(position.unwrap_or(NO_PLAYHEAD_STEP), Ordering::Relaxed);
        }
    }
}

/// Reads a published playhead set back out on the UI thread.
pub(crate) fn snapshot(steps: &[AtomicUsize; SEQUENCE_TYPE_COUNT]) -> SequencePlayheads {
    SequencePlayheads::from_steps(std::array::from_fn(|index| {
        let step = steps[index].load(Ordering::Relaxed);
        (step != NO_PLAYHEAD_STEP).then_some(step)
    }))
}

/// The step a running player is on, or `None` when nothing should be highlighted.
fn play_step(player: &SequencePlayer, sequence: &Sequence, enabled: bool) -> Option<usize> {
    if enabled
        && !sequence.is_empty()
        && player.state == SeqState::Running
        && player.pos < sequence.len()
    {
        Some(player.pos)
    } else {
        None
    }
}
