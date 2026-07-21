// `sflt-common` contains common functionality between both SFLT frontends,
// such as configuration handling, UI logic, and cross-instance SF2 sharing.

pub mod config;
pub mod fx;
pub mod gui;

use std::{collections::HashMap, fs::{self, DirEntry}, io::{self, ErrorKind}, ops::Deref, path::{Path, PathBuf}, str::FromStr, sync::{Arc, LazyLock, Mutex, Weak, atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering}}};

use arc_swap::ArcSwapOption;

pub use baseview;
pub use native_dialog;
pub use ringbuf;
pub use sfl2;
use sfl2::{hi::EngineTweak, scala::{KeyboardMapping, Tuning}};

use crate::{config::CONFIG, fx::{ChorusMode, ReverbMode}, sfl2::{lo::ResampleMethod, hi::SoundFont}};

pub static SOUNDFONT_DB: LazyLock<Arc<Mutex<HashMap<PathBuf, Weak<SoundFont>>>>> = std::sync::LazyLock::new(||Arc::default());

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[repr(i32)]
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum SfltParam {
	ReverbSend = 0,
	ChorusSend = 1,
	Modulation = 2,
	Env2Attack = 3,
	Env2Decay = 4,
	Env2Sustain = 5,
	Env2Release = 6,
	Lfo2Delay = 7,
	Lfo2Amount = 8,
	Lfo2Speed = 9,
	InitialFilterFc = 10,
	PatchNumber = 11,
	BankNumber = 12,
	ParameterMax = 13,
}

pub const PARAM_NAMES: [&'static str; SfltParam::ParameterMax as usize] = [
	"Reverb send level (multiplier)",
	"Chorus send level (multiplier)",
	"Modulation",
	"Env 2 attack time",
	"Env 2 decay time",
	"Env 2 sustain level",
	"Env 2 release time",
	"LFO 2 predelay",
	"LFO 2 amount",
	"LFO 2 speed",
	"Initial filter cutoff",
	"Patch number",
	"Bank number"
];

impl TryFrom<i32> for SfltParam {
	type Error = ();

	fn try_from(i: i32) -> Result<SfltParam, ()> {
		if i >= 0 && i <= SfltParam::ParameterMax as i32 {
			Ok(unsafe { std::mem::transmute(i) })
		} else {
			Err(())
		}
	}
}

#[derive(Debug, Default)]
pub struct DataLinkage {
	pub data: AtomicU32,
}

impl DataLinkage {
	pub const fn new() -> Self {
		DataLinkage {
			data: AtomicU32::new(0)
		}
	}

	pub fn get_param_mask(param: SfltParam) -> u32 {
		1 << (param as u32)
	}

	pub fn is_linked(&self, param: SfltParam) -> bool {
		(self.data.load(Ordering::Relaxed) & Self::get_param_mask(param)) != 0
	}

	pub fn set_linked(&self, param: SfltParam, value: bool) {
		if value {
			self.data.fetch_or(Self::get_param_mask(param), Ordering::Relaxed);
		} else {
			self.data.fetch_and(!Self::get_param_mask(param), Ordering::Relaxed);
		}
	}
}

#[derive(Debug)]
pub struct SfltSharedState {
	pub primary: SfltChannelState,
	pub multi_states: [SfltChannelState; 16],
	pub multi_timbral: AtomicBool,
	pub engine_tweaks: AtomicU64,
}

#[derive(Debug)]
pub struct SfltChannelState {
	params: [AtomicU32; SfltParam::ParameterMax as usize],
	pub filter_bypass: AtomicBool,
	pub resampling: AtomicI32,
	pub chorus: AtomicI32,
	pub reverb: AtomicI32,
	pub chorus_out: AtomicI32,
	pub reverb_out: AtomicI32,
	pub main_out: AtomicI32,
	pub sf: ArcSwapOption<SoundFont>,
	pub scl: ArcSwapOption<(String, Tuning)>,
	pub kbm: ArcSwapOption<(String, KeyboardMapping)>,
	pub linkage: DataLinkage,
	pub zone_remapping: AtomicBool,
	pub smart_pitching: AtomicBool,
}

impl SfltSharedState {
	pub fn set_param(&self, param: SfltParam, val: f32, channel: Option<u8>) {
		self.channel_state(channel).params[param as usize].store(val.to_bits(), Ordering::Relaxed)
	}
}

pub trait SfltStore: Clone {
	fn param(&self, param: SfltParam, channel: Option<u8>) -> f32;
	fn filter_bypass(&self, channel: Option<u8>) -> bool;
	fn resampling(&self, channel: Option<u8>) -> ResampleMethod;
	fn chorus(&self, channel: Option<u8>) -> ChorusMode;
	fn reverb(&self, channel: Option<u8>) -> ReverbMode;
	fn multi_timbral(&self) -> bool;
	fn soundfont(&self, channel: Option<u8>) -> impl Deref<Target = Option<Arc<SoundFont>>>;

	fn reverb_out(&self, channel: Option<u8>) -> i32;
	fn chorus_out(&self, channel: Option<u8>) -> i32;

	fn main_out(&self, channel: Option<u8>) -> i32;

	fn zone_remapping(&self, channel: Option<u8>) -> bool;
	fn smart_pitching(&self, channel: Option<u8>) -> bool;

	fn linkage(&self, channel: u8) -> &DataLinkage;

	fn bend_range(&self) -> f32 {
		2.0
	}

	fn polyphony(&self) -> u32 {
		32
	}

	fn monophonic(&self) -> bool {
		false
	}

	fn glide_time(&self) -> f32 {
		0.0
	}

	fn master_tuning(&self) -> f32 {
		0.0
	}

	fn tweak(&self, tweak: EngineTweak) -> bool;
}

impl SfltSharedState {
	pub fn channel_state(&self, channel: Option<u8>) -> &SfltChannelState {
		if let Some(channel) = channel {
			&self.multi_states[channel as usize]
		} else {
			&self.primary
		}
	}
}

impl SfltChannelState {
	const fn new(perc: bool) -> Self {
		Self {
			params: [
				AtomicU32::new((-1.0f32).to_bits()),
				AtomicU32::new((-1.0f32).to_bits()),
				AtomicU32::new((-1.0f32).to_bits()),
				AtomicU32::new((-1.0f32).to_bits()),
				AtomicU32::new((-1.0f32).to_bits()),
				AtomicU32::new((-1.0f32).to_bits()),
				AtomicU32::new((-1.0f32).to_bits()),
				AtomicU32::new((-1.0f32).to_bits()),
				AtomicU32::new((-1.0f32).to_bits()),
				AtomicU32::new((-1.0f32).to_bits()),
				AtomicU32::new((-1.0f32).to_bits()),
				AtomicU32::new((0.0f32).to_bits()),
				AtomicU32::new(if perc { 128.0f32 } else { 0.0f32 }.to_bits()),
			],
			filter_bypass: AtomicBool::new(false),
			resampling: AtomicI32::new(sfl2::lo::ResampleMethod::Linear as i32),
			chorus: AtomicI32::new(fx::ChorusMode::Disabled as i32),
			reverb: AtomicI32::new(fx::ReverbMode::Disabled as i32),
			chorus_out: AtomicI32::new(0),
			reverb_out: AtomicI32::new(0),
			main_out: AtomicI32::new(0),
			sf: ArcSwapOption::const_empty(),
			scl: ArcSwapOption::const_empty(),
			kbm: ArcSwapOption::const_empty(),
			zone_remapping: AtomicBool::new(false),
			smart_pitching: AtomicBool::new(false),
			linkage: DataLinkage::new(),
		}
	}
}

impl SfltStore for Arc<SfltSharedState> {
	fn soundfont(&self, channel: Option<u8>) -> impl Deref<Target = Option<Arc<SoundFont>>> {
		self.channel_state(channel).sf.load()
	}

	fn param(&self, param: SfltParam, channel: Option<u8>) -> f32 {
		let channel_state = self.channel_state(channel);

		if channel_state.linkage.is_linked(param) {
			f32::from_bits(self.primary.params[param as usize].load(Ordering::Relaxed))
		} else {
			f32::from_bits(channel_state.params[param as usize].load(Ordering::Relaxed))
		}
	}

	fn multi_timbral(&self) -> bool {
		self.multi_timbral.load(Ordering::Relaxed)
	}

	fn filter_bypass(&self, channel: Option<u8>) -> bool {
		self.channel_state(channel).filter_bypass.load(Ordering::Relaxed)
	}

	fn resampling(&self, channel: Option<u8>) -> ResampleMethod {
		ResampleMethod::try_from(self.channel_state(channel).resampling.load(Ordering::Relaxed)).unwrap_or(ResampleMethod::Linear)
	}

	fn chorus(&self, channel: Option<u8>) -> ChorusMode {
		ChorusMode::from_raw(self.channel_state(channel).chorus.load(Ordering::Relaxed)).unwrap_or(ChorusMode::Disabled)
	}

	fn reverb(&self, channel: Option<u8>) -> ReverbMode {
		ReverbMode::from_raw(self.channel_state(channel).reverb.load(Ordering::Relaxed)).unwrap_or(ReverbMode::Disabled)
	}

	fn chorus_out(&self, channel: Option<u8>) -> i32 {
		self.channel_state(channel).chorus_out.load(Ordering::Relaxed)
	}

	fn reverb_out(&self, channel: Option<u8>) -> i32 {
		self.channel_state(channel).reverb_out.load(Ordering::Relaxed)
	}

	fn main_out(&self, channel: Option<u8>) -> i32 {
		self.channel_state(channel).main_out.load(Ordering::Relaxed)
	}

	fn linkage(&self, channel: u8) -> &DataLinkage {
		&self.multi_states[channel as usize].linkage
	}

	fn tweak(&self, tweak: EngineTweak) -> bool {
		(self.engine_tweaks.load(Ordering::Relaxed) & (1 << tweak as u64)) != 0
	}

	fn zone_remapping(&self, channel: Option<u8>) -> bool {
		self.channel_state(channel).zone_remapping.load(Ordering::Relaxed)
	}

	fn smart_pitching(&self, channel: Option<u8>) -> bool {
		self.channel_state(channel).smart_pitching.load(Ordering::Relaxed)
	}
}

impl SfltSharedState {
	pub fn new() -> Self {
		SfltSharedState {
			primary: SfltChannelState::new(false),
			multi_timbral: AtomicBool::new(false),
			multi_states: [
				SfltChannelState::new(false),
				SfltChannelState::new(false),
				SfltChannelState::new(false),
				SfltChannelState::new(false),
				SfltChannelState::new(false),
				SfltChannelState::new(false),
				SfltChannelState::new(false),
				SfltChannelState::new(false),
				SfltChannelState::new(false),
				SfltChannelState::new(true), // percussion by default on channel 10
				SfltChannelState::new(false),
				SfltChannelState::new(false),
				SfltChannelState::new(false),
				SfltChannelState::new(false),
				SfltChannelState::new(false),
				SfltChannelState::new(false),
			],

			engine_tweaks: {
				let mut result = 0;

				for i in 0..EngineTweak::EngineTweakMax as usize {
					result |= (EngineTweak::newest_default(i) as u64) << i;
				}

				AtomicU64::new(result)
			}
		}
	}
}

pub fn unclamped_param_allowed(param: SfltParam) -> bool {
	match param {
		SfltParam::Env2Attack | SfltParam::Env2Decay | SfltParam::Env2Release => true,
		_ => false,
	}
}

pub fn value_from_text<T: FromStr + Ord>(text: &str, min: T, max: T, default: T) -> T {
	if let Ok(result) = text.parse::<T>() {
		result.clamp(min, max)
	} else {
		default
	}
}

pub fn param_value_from_text(param: SfltParam, text: &str) -> i32 {
	if let Ok(mut result) = text.parse::<i32>() {
		if param == SfltParam::Lfo2Amount {
			result += 129;
		}

		result = if unclamped_param_allowed(param) {
			result.max(0)
		} else {
			result.clamp(0, get_param_max_value(param))
		};

		result
	} else {
		get_param_min_value(param)
	}
}

pub fn get_param_value_text(param: SfltParam, value: i32) -> String {
	if value == -1 {
		return "Disabled".into()
	}

	match param {
		SfltParam::ChorusSend | SfltParam::ReverbSend => format!("{value}%"),

		//SfltParam::Env2Attack => format!("{value} ({:.1}s)", value as f32 / 1000.0),
		//SfltParam::Env2Decay | SfltParam::Env2Release => format!("{value} ({:.1}s)", value as f32 / 250.0),

		SfltParam::Lfo2Amount => (value - 129).to_string(),

		_ => value.to_string()
	}
}

pub fn get_param_min_value(param: SfltParam) -> i32 {
	match param {
		SfltParam::BankNumber | SfltParam::PatchNumber => 0,
		_ => -1,
	}
}

pub fn get_param_max_value(param: SfltParam) -> i32 {
	match param {
		SfltParam::Env2Attack |
		SfltParam::Env2Decay |
		SfltParam::Env2Release
			=> 5940,

		SfltParam::Lfo2Delay => 5900,

		SfltParam::Lfo2Amount => 256,

		SfltParam::ReverbSend |
		SfltParam::ChorusSend
			=> 200,

		SfltParam::PatchNumber => 127,
		SfltParam::BankNumber => 128,

		_ => 127,
	}
}

// this effectively gets called twice; first by the frontend to check whether
// it needs to queue a threaded load, then once the load lock is held, to make
// sure the soundfont wasn't inserted into the database since the first call
fn try_fetch_soundfont_inner(path: &Path, db: &HashMap<PathBuf, Weak<SoundFont>>) -> Option<Arc<SoundFont>> {
	if let Some(sf) = db.get(path) {
		sf.upgrade()
	} else {
		None
	}
}


pub fn try_fetch_soundfont(path: &Path) -> Option<Arc<SoundFont>> {
	try_fetch_soundfont_inner(path, &SOUNDFONT_DB.lock().unwrap())
}


pub fn load_soundfont_into_db(path: PathBuf, load_progress: Arc<AtomicU32>) -> std::io::Result<Arc<SoundFont>> {
	load_progress.store(0, Ordering::SeqCst);

	let mut lock = SOUNDFONT_DB.lock().unwrap();

	// two plugins may try loading the same SoundFont at the same time,
	// possibly causing orphan SoundFont duplicates. we re-check the db
	// here to avoid the race condition
	if let Some(fetched) = try_fetch_soundfont_inner(&path, &lock) {
		return Ok(fetched)
	}

	let data = sfl2::lo::Sf2Data::load_from_file(
		path.clone(),
		CONFIG.read().unwrap().use_24_bit_samples,
		load_progress
	)?;

	let sf = Arc::new(SoundFont::from_sf2_data(data));
	lock.insert(path, Arc::downgrade(&sf));

	Ok(sf)
}

pub fn locate_soundfont(mut path: PathBuf) -> io::Result<PathBuf> {
	if path.exists() {
		return Ok(path)
	}

	let search_path = CONFIG.read().unwrap().library_path.clone();

	if !search_path.exists() {
		return Err(io::Error::new(ErrorKind::InvalidFilename, "library directory does not exist!"))
	}
	
	let filename = path.clone();
	let Some(filename) = filename.file_name() else {
		return Err(io::Error::new(ErrorKind::InvalidFilename, "not a SoundFont file!"))
	};

	let pathref = &mut path;

	visit_dirs(&search_path, &mut move |file| {
		if file.file_name() == filename {
			*pathref = file.path();
			true
		} else {
			false
		}
	})?;

	Ok(path)
}

fn visit_dirs(dir: &Path, cb: &mut impl FnMut(&DirEntry) -> bool) -> io::Result<bool> {
	if dir.is_dir() {
		for entry in fs::read_dir(dir)? {
			let entry = entry?;
			let path = entry.path();

			if path.is_dir() {
				if visit_dirs(&path, cb)? {
					return Ok(true)
				}
			} else {
				if cb(&entry) {
					return Ok(true)
				};
			}
		}
	}
	Ok(false)
}

pub fn find_valid_preset(sf: &SoundFont, bank: u8, patch: u8) -> Option<(u8, u8)> {
	if let Some(preset) = sf.get_preset_idx(bank, patch) {
		return Some((sf.presets[preset].bank as u8, sf.presets[preset].preset as u8))
	}

	for i in 0..128 {
		if let Some(lowest) = sf.preset_map.get(&(bank, i)) {
			return Some((sf.presets[*lowest].bank as u8, sf.presets[*lowest].preset as u8))
		}
	}

	None
}

pub fn get_library_path() -> PathBuf {
	let mut path = CONFIG.read().unwrap().library_path.to_string_lossy().into_owned();

	if path.ends_with("/") {
		path.pop();
	}

	#[cfg(target_os = "windows")]
	{
		// windows hack
		path = path.replace("/", "\\");
	}

	path.into()
}

pub fn tweak_mask_compat() -> u64 {
	let mut result = 0;

	for i in 0..EngineTweak::EngineTweakMax as usize {
		if EngineTweak::compat_default(i) {
			result |= 1 << i;
		}
	}

	result
}

pub fn tweak_mask_newest() -> u64 {
	let mut result = 0;

	for i in 0..EngineTweak::EngineTweakMax as usize {
		if EngineTweak::newest_default(i) {
			result |= 1 << i;
		}
	}
	result
}

pub fn get_error_string(e: std::io::Error) -> &'static str {
	match e.kind() {
		ErrorKind::OutOfMemory => "Out of memory",

		ErrorKind::InvalidFilename => "File not found",

		ErrorKind::PermissionDenied => "Permission denied",

		ErrorKind::UnexpectedEof | ErrorKind::InvalidData => "Invalid SF2",

		_ => "Failed to load SF2"
	}
}