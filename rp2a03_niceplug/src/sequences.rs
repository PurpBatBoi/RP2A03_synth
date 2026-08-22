//! `rp2a03_niceplug\src\sequences.rs`
//! The sequence-state bridges between the editor and the audio thread:
//! `SharedSequencesHandle` is the GUI-side seam (a `triple_buffer` pair
//! underneath, so `process` never takes a lock), `SequenceCache` pulls
//! envelope data down for playback on the audio side, and
//! `PlayheadPublisher`/`SequenceIndexPublisher` push state back up for
//! display.

use basedrop::{Collector, Handle as DropHandle, Shared};
use nice_plug::params::persist::PersistentField;
use parking_lot::Mutex;
use rp2a03_common::{
    ActiveSequences, FDS_WAVE_LEN, Lane, MidiHandler, NO_PLAYHEAD_STEP, SequencePlayheads,
    SequenceReload, SharedSequences, WaveSlots, fds_wave_from_slot,
};
use rp2a03_core::sequencer::{SeqState, Sequence, SequencePlayer};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use triple_buffer::{Input, Output, triple_buffer};

/// The resolved, ready-to-play FDS wave table — computed on the GUI thread
/// from `SharedSequences::wave_slots()` and handed to the audio thread as a
/// refcounted pointer, so `SequenceCache::refresh_waves` never resizes or
/// recomputes anything on the audio thread itself (see M7 step 43).
#[derive(Clone, Default)]
pub struct ResolvedFdsWaves {
    pub(crate) waves: Option<Shared<Vec<[u8; FDS_WAVE_LEN]>>>,
    pub(crate) current_slot: usize,
}

/// The GUI-side half of the lock-free seam: a live, directly-editable
/// `SharedSequences` the editor mutates in place exactly as before, plus the
/// `triple_buffer` writers that publish a snapshot to the audio thread
/// whenever `SharedSequencesGuard` drops and finds the revision (or the wave
/// slots) actually changed.
struct GuiSide {
    master: SharedSequences,
    input: Input<SharedSequences>,
    published_revision: u64,
    published_wave_slots: WaveSlots,
    wave_input: Input<ResolvedFdsWaves>,
    wave_collector: Collector,
    wave_handle: DropHandle,
}

impl GuiSide {
    /// Publishes `master` and/or the resolved wave table if either changed
    /// since the last publish. Called on every `SharedSequencesGuard` drop —
    /// cheap no-op on an idle frame, since both checks are plain comparisons.
    fn publish_if_dirty(&mut self) {
        if self.master.revision() != self.published_revision {
            self.publish_master();
        }
        if *self.master.wave_slots() != self.published_wave_slots {
            self.publish_waves();
        }
    }

    fn publish_master(&mut self) {
        self.input.write(self.master.clone());
        self.published_revision = self.master.revision();
    }

    /// Resolves every wave slot into `[u8; FDS_WAVE_LEN]` once, here on the
    /// GUI thread, and publishes the result as a `basedrop::Shared` pointer —
    /// the audio thread only ever clones the pointer (a refcount bump).
    /// `wave_collector.collect()` is where the *previous* published buffer's
    /// actual deallocation happens, also off the audio thread.
    fn publish_waves(&mut self) {
        let slots = self.master.wave_slots();
        let all = slots.slots();
        let waves = if all.is_empty() {
            None
        } else {
            let resolved: Vec<[u8; FDS_WAVE_LEN]> = all
                .iter()
                .map(|slot| fds_wave_from_slot(slot.data()))
                .collect();
            Some(Shared::new(&self.wave_handle, resolved))
        };
        self.wave_input.write(ResolvedFdsWaves {
            waves,
            current_slot: slots.current_slot(),
        });
        self.published_wave_slots = slots.clone();
        self.wave_collector.collect();
    }
}

/// Owns the GUI-side seam. `#[persist = "envelope_data"]` requires this type
/// to implement `PersistentField<SharedSequences>`, which is what makes host
/// session save/load still round-trip through serde.
pub struct SharedSequencesHandle {
    inner: Mutex<GuiSide>,
}

impl SharedSequencesHandle {
    /// Builds the handle and both `triple_buffer` reader halves together, so
    /// the two sides can never be constructed out of sync. Not `Arc`-wrapped
    /// itself — `Rp2a03Params` (which owns this) is already handed out as
    /// `Arc<Rp2a03Params>`, and wrapping it a second time here would make
    /// `PersistentField` unimplementable: `Arc` isn't a fundamental type, so
    /// `impl ForeignTrait for Arc<LocalType>` is an orphan-rule violation
    /// from a downstream crate.
    pub(crate) fn new() -> (Self, Output<SharedSequences>, Output<ResolvedFdsWaves>) {
        let master = SharedSequences::default();
        let published_revision = master.revision();
        let published_wave_slots = master.wave_slots().clone();
        let (input, output) = triple_buffer(&master);
        let (wave_input, wave_output) = triple_buffer(&ResolvedFdsWaves::default());
        let wave_collector = Collector::new();
        let wave_handle = wave_collector.handle();

        let handle = Self {
            inner: Mutex::new(GuiSide {
                master,
                input,
                published_revision,
                published_wave_slots,
                wave_input,
                wave_collector,
                wave_handle,
            }),
        };

        (handle, output, wave_output)
    }

    /// Locks the GUI-side master copy for editing. The returned guard derefs
    /// to `SharedSequences` exactly like the old `parking_lot::MutexGuard`
    /// did, so call sites barely change; on drop it publishes to the audio
    /// thread if anything actually changed.
    pub(crate) fn lock_master(&self) -> SharedSequencesGuard<'_> {
        SharedSequencesGuard {
            guard: self.inner.lock(),
        }
    }
}

pub struct SharedSequencesGuard<'a> {
    guard: parking_lot::MutexGuard<'a, GuiSide>,
}

impl Deref for SharedSequencesGuard<'_> {
    type Target = SharedSequences;

    fn deref(&self) -> &SharedSequences {
        &self.guard.master
    }
}

impl DerefMut for SharedSequencesGuard<'_> {
    fn deref_mut(&mut self) -> &mut SharedSequences {
        &mut self.guard.master
    }
}

impl Drop for SharedSequencesGuard<'_> {
    fn drop(&mut self) {
        self.guard.publish_if_dirty();
    }
}

impl PersistentField<'_, SharedSequences> for SharedSequencesHandle {
    fn set(&self, new_value: SharedSequences) {
        let mut guard = self.inner.lock();
        guard.master = new_value;
        // Unconditional, not revision-gated: a freshly deserialized value's
        // revision could coincidentally match what's already published, but
        // the audio thread still needs the actual loaded content.
        guard.publish_master();
        guard.publish_waves();
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&SharedSequences) -> R,
    {
        let guard = self.inner.lock();
        f(&guard.master)
    }
}

#[derive(Default)]
pub struct SequenceCache {
    pub(crate) active: ActiveSequences,

    key: Option<(usize, u64)>,
}

impl SequenceCache {
    /// Pulls playback data down for `sequence_index`. Gated on a single
    /// `(sequence_index, revision)` key — an idle block (nothing edited,
    /// same slot) returns `None` immediately without touching any field, GUI
    /// or audio. Reads by explicit index (`sequence_at`/`sequence_enabled_at`),
    /// never through the editor's own "currently selected" state, so this
    /// never needs to write back into the GUI-side state.
    pub(crate) fn refresh(
        &mut self,
        shared: &mut Output<SharedSequences>,
        waves: &mut Output<ResolvedFdsWaves>,
        sequence_index: usize,
    ) -> Option<SequenceReload> {
        let data = shared.read();

        let revision = data.revision();
        if self.key == Some((sequence_index, revision)) {
            return None;
        }

        let slot_changed = self.key.map(|(index, _)| index) != Some(sequence_index);

        self.active.wavesynth = data.wavesynth(sequence_index);
        self.active
            .fds_settings
            .clone_from(data.fds_settings(sequence_index));
        self.refresh_waves(waves);

        let mut reload = SequenceReload::default();
        if slot_changed {
            for lane in Lane::ALL {
                reload[lane] = self.active.seq[lane] != *data.sequence_at(lane, sequence_index);
            }
        }

        for lane in Lane::ALL {
            self.active.seq[lane].clone_from(data.sequence_at(lane, sequence_index));
            self.active.enabled[lane] = data.sequence_enabled_at(lane, sequence_index);
        }
        self.key = Some((sequence_index, revision));

        slot_changed.then_some(reload)
    }

    /// Cheap on the audio thread: clones a `basedrop::Shared` pointer (a
    /// refcount bump) instead of resizing/recomputing the wave vector.
    fn refresh_waves(&mut self, waves: &mut Output<ResolvedFdsWaves>) {
        let resolved = waves.read();
        self.active.fds_waves.clone_from(&resolved.waves);
        self.active.fds_current_wave = resolved.current_slot;
    }
}

pub struct PlayheadPublisher {
    steps: Arc<[AtomicUsize; Lane::COUNT]>,
}

impl PlayheadPublisher {
    pub(crate) fn new() -> Self {
        Self {
            steps: Arc::new(std::array::from_fn(|_| AtomicUsize::new(NO_PLAYHEAD_STEP))),
        }
    }

    pub(crate) fn handle(&self) -> Arc<[AtomicUsize; Lane::COUNT]> {
        self.steps.clone()
    }

    pub(crate) fn clear(&self) {
        for step in self.steps.iter() {
            step.store(NO_PLAYHEAD_STEP, Ordering::Relaxed);
        }
    }

    pub(crate) fn publish(&self, handler: &MidiHandler, seqs: &ActiveSequences) {
        for lane in Lane::ALL {
            let position = play_step(
                &handler.seq_players[lane],
                &seqs.seq[lane],
                seqs.enabled[lane],
            );
            self.steps[lane as usize]
                .store(position.unwrap_or(NO_PLAYHEAD_STEP), Ordering::Relaxed);
        }
    }
}

/// Publishes the sequence slot actually driving playback — which can differ
/// from the host's Index parameter under a MIDI Program Change override —
/// so the editor can display it without the audio thread writing into the
/// GUI-side state (that used to happen through `SequenceCache::refresh`; see
/// M4 step 26).
pub struct SequenceIndexPublisher {
    index: Arc<AtomicUsize>,
}

impl SequenceIndexPublisher {
    pub(crate) fn new() -> Self {
        Self {
            index: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn handle(&self) -> Arc<AtomicUsize> {
        self.index.clone()
    }

    pub(crate) fn clear(&self) {
        self.index.store(0, Ordering::Relaxed);
    }

    pub(crate) fn publish(&self, index: usize) {
        self.index.store(index, Ordering::Relaxed);
    }
}

pub fn snapshot(steps: &[AtomicUsize; Lane::COUNT]) -> SequencePlayheads {
    SequencePlayheads::from_steps(std::array::from_fn(|index| {
        let step = steps[index].load(Ordering::Relaxed);
        (step != NO_PLAYHEAD_STEP).then_some(step)
    }))
}

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

#[cfg(test)]
mod tests {
    //! M4 step 21: proves an idle block (nothing edited since the last
    //! `refresh`) takes the early-return path and performs zero `Sequence`
    //! clones — not just "no allocation", the copy loop must not run at all.
    //! A sentinel value planted directly in `active.seq` would be silently
    //! overwritten by a real `clone_from` call; its survival is the proof.

    use super::*;

    #[test]
    fn an_idle_block_performs_zero_sequence_clones() {
        let (_input, mut output) = triple_buffer(&SharedSequences::default());
        let (_wave_input, mut wave_output) = triple_buffer(&ResolvedFdsWaves::default());
        let mut cache = SequenceCache::default();

        cache.refresh(&mut output, &mut wave_output, 0);

        let sentinel = Sequence {
            values: vec![99, 99, 99],
            ..Sequence::default()
        };
        cache.active.seq[Lane::Vol] = sentinel.clone();

        let reload = cache.refresh(&mut output, &mut wave_output, 0);

        assert!(reload.is_none(), "an idle block must not report a reload");
        assert_eq!(
            cache.active.seq[Lane::Vol],
            sentinel,
            "the per-lane copy loop ran on an idle block and overwrote the sentinel"
        );
    }
}
