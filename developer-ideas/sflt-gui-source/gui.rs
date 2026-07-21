use std::{borrow::Cow, ffi::OsString, fs, path::PathBuf, sync::{atomic::{AtomicU32, Ordering}, mpsc::{Receiver, SyncSender}, Arc, Mutex, RwLock, Weak}};

use baseview::{WindowHandle, WindowOpenOptions};
use egui::{Align, Button, Checkbox, Color32, Context, CursorIcon, FontData, FontDefinitions, FontFamily, FontId, Image, Label, Layout, Modal, Modifiers, Popup, Pos2, Rect, RectAlign, Response, RichText, Sense, Separator, SetOpenCommand, Style, TextEdit, TextStyle, TextWrapMode, TextureOptions, Ui, Vec2, containers::menu::SubMenuButton, include_image};
use egui_baseview::GraphicsConfig;
use egui_extras::{Column, TableBuilder};
use knob::Knob;
use queues::{queue, IsQueue, Queue};
use rand::random;
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
use ringbuf::{traits::{Producer, Split}, HeapRb};
use sfl2::{OGG_FLAG, SM24_FLAG, hi::EngineTweak};

#[cfg(target_os = "windows")]
use windows::{core::PCSTR, Win32::{Foundation::HWND, Storage::FileSystem::{GetDriveTypeA, GetFileAttributesA, GetLogicalDrives, INVALID_FILE_ATTRIBUTES}}};

#[cfg(feature = "wgpu")]
use egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew, wgpu::{Backends, InstanceDescriptor, PowerPreference}};

#[allow(unused_imports)]
use sfl2::hi::timecents_to_secs;

#[cfg(feature = "fl")]
use crate::config::RenameMode;

use crate::{SOUNDFONT_DB, SfltParam, SfltStore, VERSION, config::CONFIG, fx::{ChorusMode, ReverbMode}, get_library_path, get_param_value_text, gui::{combo_box::ComboBox, drag_value::DragValue, slider::Slider}, param_value_from_text, sfl2::lo::ResampleMethod, value_from_text};

mod combo_box;
mod drag_value;
mod knob;
mod slider;

const SFLT_LINK: &'static str = "https://estrobiologist.gumroad.com/l/sflt";
const MINUS_CHAR_STR: &'static str = "-";
const HIGHLIGHT_COLOR: Color32 = Color32::from_rgb(53, 94, 183);

pub const WINDOW_WIDTH: u32 = 370;
pub const WINDOW_HEIGHT: u32 = 395;

pub type SfltChannelFront<S> = (<HeapRb<SfltUiMessage> as Split>::Prod, Receiver<SfltEngineMessage<S>>);
pub type SfltChannelBack<S> = (<HeapRb<SfltUiMessage> as Split>::Cons, SyncSender<SfltEngineMessage<S>>);

static LOADED_SF_LIST: RwLock<Vec<PathBuf>> = RwLock::new(vec![]);
static FONT_DATA: Mutex<Weak<Vec<(String, Arc<FontData>)>>> = Mutex::new(Weak::new());
static MOTD_LIST: &[&str] = &[
	"it's pronounced \"svelte\".",
	"I'M CHORDING OUT !!",
	"I'M SFLTING OUT !!",
	"it's pronounced \"ess-eff-ell-tee\".",
	"now with extra Mother 3.sf2!",
	"go ahead. use Earthbound.sf2 bank 0 patch 19.",
	"also check out chordioid.com!",
	"they're putting PRONOUNS in the soundfont players.",
	"i'd like a word with whoever wrote the SF2 spec.",
	"fine. i'll do it myself.",
	"collect my chordioids.",
	"FUCK IM SFLTING DOWN ALL THESE STAIRS..........",
	"I WARNED YOU ABOUT SFLT BRO!!!! I TOLD YOU DOG!",
	"THE SFLT RUSE WAS A........ DISTACTION",
	"i HAVE the chord",
	"composer. i remember you're soundfonts",
	"the T stands for Transgender.",
	"SoundFonts... gone WOKE!",
	"is literally any SF2 using linked modulators?",
	"SFLTing as usual i see...",
	"IT'S ME BOY I'M THE SF2",
	"SPEAKING TO YOU INSIDE YOUR BRAIN",
	"only the best for your undertale lookin' ass.",
	"brought to you by the spectre of communism.",
];

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum FileEntryKind {
	Parent,
	Dir,
	File,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum SettingsSubpage {
	#[cfg(not(feature = "fl"))]
	Instance,
	Engine,
	Global,
}

enum SfltUiPage {
	Main,
	FileBrowser { entries: Vec<PathBuf>, filtered: Vec<(OsString, FileEntryKind)> },
	PresetBrowser { filtered: Vec<usize> },
	Settings,
	About,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum PresetSortOrder {
	Alphabetical,
	Preset,
	Bank
}

#[derive(PartialEq, Clone)]
pub enum HintText {
	Hover(&'static str),
	ParamHover(SfltParam, f32),
	Edit(SfltParam),
	Custom(String),
	None
}

pub trait MessageHandler {
	fn handle_message(&mut self, msg: SfltUiMessage) -> Option<SfltUiMessage> { Some(msg) }
}

impl MessageHandler for () {}

pub enum SfltEngineMessage<S: SfltStore> {
	SoundFontLoadStarted,
	SoundFontLoadFinished(Result<(), &'static str>),
	FlSetFocus(bool),
	WindowHandleCreated(RawWindowHandle),
	StateLoaded(S),
}

#[derive(PartialEq, Eq, Debug, Copy, Clone)]
enum ActiveControl {
	Param(SfltParam),
}

unsafe impl<S: SfltStore> Send for SfltEngineMessage<S> {}

#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub enum AuxFileKind {
	Tuning,
	Mapping,
	KeyNames,
}

#[derive(Clone)]
pub enum SfltUiMessage {
	LibraryPathLocator,
	SoundFontLocator(Option<u8>),
	FxChanged(ReverbMode, ChorusMode, Option<u8>),
	ParamChanged(SfltParam, f32, Option<u8>),
	SoundFontChanged(PathBuf, Option<u8>),
	SoundFontCleared(Option<u8>),
	ParamMenuOpened(SfltParam, RawWindowHandle),
	ResamplingChanged(ResampleMethod, Option<u8>),
	FilterBypassChanged(bool, Option<u8>),
	BendRangeChanged(f32),
	HintChanged(HintText),
	SetPreviewNote(Option<usize>),
	PolyphonyChanged(u32),
	FxTracksChanged(i32, i32, Option<u8>),
	MonophonicChanged(bool),
	GlideTimeChanged(f32),
	MasterTuningChanged(f32),
	MultiTimbralChanged(bool),

	LoadAuxFile(AuxFileKind, PathBuf, Option<u8>),
	LocateAuxFile(AuxFileKind, Option<u8>),
	ClearAuxFile(AuxFileKind, Option<u8>),

	ZoneRemappingChanged(bool, Option<u8>),
	SmartPitchingChanged(bool, Option<u8>),
	
	MainOutChanged(i32, Option<u8>),
	EngineTweakChanged(EngineTweak, bool),
}

unsafe impl Send for SfltUiMessage {}

pub struct SfltUiConnection<T: MessageHandler, S: SfltStore> {
	pub handler: T,
	channel: SfltChannelFront<S>,
	msg_buffer: queues::Queue<SfltUiMessage>,
}

pub struct SfltUiState<T: MessageHandler, S: SfltStore> {
	state: S,
	page: SfltUiPage,
	text_edit_buf: String,
	set_value_buf: String,
	page_dirty: bool,
	loading: bool,
	preset_scroll_list: Vec<usize>,
	preset_sort_order: PresetSortOrder,
	preset_sort_ascending: bool,
	file_sort_ascending: bool,
	pub own_handle: Option<RawWindowHandle>, // fuck it
	focused: bool,
	queued_mouse_offset: Option<Vec2>,
	mouse_offset: Vec2,
	drag_overflow: f32,
	active_drag: Option<ActiveControl>,
	did_mouse_jump: bool,
	last_hint: HintText,
	pub connection: SfltUiConnection<T, S>,
	capture_mouse: bool,
	preview_note: Option<usize>,
	active_channel: u8,
	prev_active_channel: Option<u8>,
	motd: &'static str,
	current_path: Option<PathBuf>,
	settings_subpage: SettingsSubpage,

	logo_about: Image<'static>,
	logo: Image<'static>,

	fonts: Option<Arc<Vec<(String, Arc<FontData>)>>>,

	load_error: Option<&'static str>,
	load_progress: Arc<AtomicU32>,
}

unsafe impl<T: MessageHandler, S: SfltStore> Send for SfltUiState<T, S> {}

impl<T: MessageHandler, S: SfltStore> SfltUiState<T, S> {
	pub fn new(channel: SfltChannelFront<S>, state: S, handler: T, load_progress: Arc<AtomicU32>) -> Self {
		Self {
			state,
			page: SfltUiPage::Main,
			text_edit_buf: String::new(),
			set_value_buf: String::new(),
			page_dirty: true,
			preset_scroll_list: vec![],
			preset_sort_order: PresetSortOrder::Bank,
			preset_sort_ascending: true,
			file_sort_ascending: true,
			own_handle: None,
			focused: false,
			queued_mouse_offset: None,
			mouse_offset: Vec2::ZERO,
			drag_overflow: 0.0,
			active_drag: None,
			did_mouse_jump: false,
			loading: false,
			last_hint: HintText::None,
			connection: SfltUiConnection {
				channel,
				msg_buffer: queue![],
				handler,
			},
			capture_mouse: false,
			preview_note: None,
			active_channel: 0,
			prev_active_channel: None,
			motd: MOTD_LIST[0],

			current_path: Some(get_library_path()),
			settings_subpage: SettingsSubpage::Global,

			logo_about:
				Image::new(include_image!("../../logo-about.png"))
					.max_width(250.0)
					.texture_options(TextureOptions {
						magnification: egui::TextureFilter::Linear,
						minification: egui::TextureFilter::Linear,
						wrap_mode: egui::TextureWrapMode::ClampToEdge,
						mipmap_mode: None,
					})
					.show_loading_spinner(false),

			logo: Image::new(include_image!("../../logo.png"))
					.max_size(Vec2::new(
						182.0,
						79.0
					))
					.texture_options(TextureOptions {
						magnification: egui::TextureFilter::Linear,
						minification: egui::TextureFilter::Linear,
						wrap_mode: egui::TextureWrapMode::ClampToEdge,
						mipmap_mode: None,
					})
					.show_loading_spinner(false),

			fonts: None,

			load_error: None,
			load_progress,
		}
	}

	fn push_message(&mut self, msg: SfltUiMessage) {
		self.connection.push_message(msg);
	}

	fn try_flush_messages(&mut self) {
		self.connection.try_flush_messages();
	}

	fn process_engine_messages(&mut self) {
		while let Ok(msg) = self.connection.channel.1.try_recv() {
			match msg {
				SfltEngineMessage::SoundFontLoadStarted => {
					self.page = SfltUiPage::Main;
					self.page_dirty = true;
					self.loading = true;
				}

				SfltEngineMessage::SoundFontLoadFinished(Ok(())) => {
					self.update_preset_scroll_list();
					self.update_browser_filter();
					self.loading = false;
					self.load_progress.store(0, Ordering::SeqCst);
				}

				SfltEngineMessage::SoundFontLoadFinished(Err(msg)) => {
					self.update_preset_scroll_list();
					self.update_browser_filter();
					self.load_error = Some(msg);
					self.loading = false;
					self.load_progress.store(0, Ordering::SeqCst);
				}

				SfltEngineMessage::FlSetFocus(focused) => {
					self.focused = focused;
				}

				SfltEngineMessage::WindowHandleCreated(raw_handle) => {
					self.own_handle = Some(raw_handle);
					self.update_preset_scroll_list();
					self.update_browser_filter();
				}

				SfltEngineMessage::StateLoaded(state) => {
					self.state = state;
				}
			}
		}
	}

	fn refresh_browser_list(&mut self) {
		match (&mut self.page, &mut self.current_path) {
			(SfltUiPage::FileBrowser { entries, .. }, Some(path)) if !path.as_os_str().is_empty() => {
				entries.clear();

				let Ok(dir) = fs::read_dir(path) else {
					return
				};

				for entry in dir {
					let Ok(entry) = entry else {
						continue
					};

					entries.push(entry.path());
				}

				self.update_browser_filter();
			}

			// not in any path, list available disk drives
			(SfltUiPage::FileBrowser { entries, .. }, path) => {
				*path = None;
				entries.clear();

				#[cfg(target_os = "windows")]
				{
					let drives = unsafe { GetLogicalDrives() };

					for i in 0..32 {
						let available = (drives & (1 << i)) != 0;

						if !available {
							continue
						};

						let mut drive_path = String::from_utf8(vec![65 + i]).unwrap();

						drive_path.push(':');

						let mut drive_chk = drive_path.clone();

						drive_chk.push('\\');
						drive_chk.push('\0');

						let drive_chk_val = unsafe { GetDriveTypeA(PCSTR(drive_chk.as_bytes().as_ptr())) };

						if drive_chk_val < 2 || drive_chk_val == 4 {
							continue
						}

						if unsafe { GetFileAttributesA(PCSTR(drive_chk.as_bytes().as_ptr())) } == INVALID_FILE_ATTRIBUTES {
							continue
						}

						entries.push(PathBuf::from(drive_path));
					}
				}

				self.update_browser_filter();
			}

			_ => {}
		}
	}

	fn update_preset_scroll_list(&mut self) {
		self.preset_scroll_list.clear();

		let sf;
		let channel = self.state.soundfont(self.get_active_channel());
		let master = self.state.soundfont(None);

		if let Some(channel) = &*channel {
			sf = channel;
		} else if let Some(master) = &*master {
			sf = master;
		} else {
			return
		}

		self.preset_scroll_list.extend(0..sf.presets.len());

		self.preset_scroll_list.sort_by(|a, b| {
			let a = &sf.presets[*a];
			let b = &sf.presets[*b];

			if a.bank == b.bank {
				a.preset.cmp(&b.preset)
			} else {
				a.bank.cmp(&b.bank)
			}
		});
	}

	fn update_browser_filter(&mut self) {
		let active_channel = self.get_active_channel();

		match &mut self.page {
			SfltUiPage::FileBrowser { entries, filtered, .. } => {
				let path = &mut self.current_path;
				filtered.clear();

				if self.text_edit_buf.is_empty() {
					#[cfg(target_os = "windows")]
					if path.is_some() {
						filtered.push(("..".into(), FileEntryKind::Parent));
					}

					#[cfg(not(target_os = "windows"))]
					if let Some(path) = path && path.parent().is_some() {
						filtered.push(("..".into(), FileEntryKind::Parent));
					}
				}

				let filter = self.text_edit_buf.to_lowercase();

				for entry in entries {
					let name;

					if let Some(filename) = entry.file_name() {
						name = filename;
					} else {
						name = entry.as_os_str();
					}

					if let Some(string) = name.to_str() {
						let string = string.to_lowercase();

						if !self.text_edit_buf.is_empty() {
							if !string.contains(&filter) {
								continue
							}
						}

						if !entry.is_dir() && !(string.contains(".sf2") || string.contains(".sf3")) {
							continue
						}
					}

					if entry.is_dir() {
						filtered.push((name.into(), FileEntryKind::Dir));
					} else {
						filtered.push((name.into(), FileEntryKind::File));
					}
				}

				filtered.sort_by(|a, b| {
					let raw = if a.1 != b.1 {
						a.1.cmp(&b.1)
					} else {
						let al = a.0.to_ascii_lowercase();
						let bl = b.0.to_ascii_lowercase();
						if al != bl {
							al.cmp(&bl)
						} else {
							a.0.cmp(&b.0)
						}
					};

					if self.file_sort_ascending {
						raw
					} else {
						raw.reverse()
					}
				});
			}

			SfltUiPage::PresetBrowser { filtered } => {
				filtered.clear();

				let filter = self.text_edit_buf.to_lowercase();

				let mut sf = self.state.soundfont(active_channel);

				if sf.is_none() && active_channel.is_some() {
					sf = self.state.soundfont(None);
				}

				if let Some(sf) = &*sf {
					for (i, preset) in sf.presets.iter().enumerate() {
						if preset.name.to_lowercase().contains(&filter) {
							filtered.push(i);
						}
					}

					filtered.sort_by(|a, b| {
						let a = &sf.presets[*a];
						let b = &sf.presets[*b];

						let raw = match self.preset_sort_order {
							PresetSortOrder::Alphabetical => {
								let aname = a.name.to_lowercase();
								let bname = b.name.to_lowercase();

								if aname != bname {
									aname.cmp(&bname)
								} else if a.bank != b.bank {
									a.bank.cmp(&b.bank)
								} else {
									a.preset.cmp(&b.preset)
								}
							}

							PresetSortOrder::Bank => {
								if a.bank != b.bank {
									a.bank.cmp(&b.bank)
								} else {
									a.preset.cmp(&b.preset)
								}
							}

							PresetSortOrder::Preset => {
								if a.preset != b.preset {
									a.preset.cmp(&b.preset)
								} else {
									a.bank.cmp(&b.bank)
								}
							}
						};

						if self.preset_sort_ascending {
							raw
						} else {
							raw.reverse()
						}
					});
				}
			}

			_ => {}
		}
	}

	fn get_active_channel(&self) -> Option<u8> {
		if self.state.multi_timbral() && self.active_channel != 0 {
			Some(self.active_channel - 1)
		} else {
			None
		}
	}
}

impl<T: MessageHandler, S: SfltStore> SfltUiConnection<T, S> {
	
	fn push_message(&mut self, msg: SfltUiMessage) {
		self.try_flush_messages();
		self.push_message_inner(msg);
	}

	fn push_message_inner(&mut self, mut msg: SfltUiMessage) -> bool {
		if let Some(returned) = self.handler.handle_message(msg) {
			msg = returned;
		} else {
			return true;
		}

		match self.channel.0.try_push(msg) {
			Ok(_) => true,

			Err(returned) => {
				let _ = self.msg_buffer.add(returned);
				false
			}
		}
	}

	fn try_flush_messages(&mut self) {
		while let Ok(queued) = self.msg_buffer.peek() {
			if !self.push_message_inner(queued) {
				return
			}

			let _ = self.msg_buffer.remove();
		}
	}

}

fn get_scroll_delta(ui: &mut Ui) -> f32 {
	ui.input(|i| {
		i.events.iter().find_map(|e| match e {
			egui::Event::MouseWheel {
				unit: _,
				delta,
				modifiers,
			} => Some(*delta * if modifiers.command { 0.1 } else { 1.0 }),

			_ => None,
		})
	}).unwrap_or(Vec2::ZERO).y
}


fn widget_scroll_behaviour<Num: egui::emath::Numeric>(ui: &mut Ui, num: &mut Num, response: &mut Response, min: f64, max: f64, speed: Option<f64>) {
	if response.hovered() {
		let delta = get_scroll_delta(ui) as f64;

		let speed = speed.unwrap_or((max - min) / 46.40625);

		if delta != 0.0 {
			*num = Num::from_f64((num.to_f64() + delta * speed).clamp(min, max));
			response.flags |= egui::response::Flags::CHANGED;
		}
	}
}


fn sflt_frame(ctx: &Context) -> egui::frame::Frame {
	egui::Frame::central_panel(&*ctx.style())
		.stroke(ctx.style().visuals.window_stroke)
		.corner_radius(4.0)
}


#[allow(unused_variables)]
fn sflt_widget_wrap<T: MessageHandler, S: SfltStore>(response: &Response, ui: &mut Ui, state: &mut SfltUiState<T, S>) {
	let Some(own_handle) = state.own_handle else {
		return
	};

	if !state.capture_mouse {
		ui.ctx().output_mut(|output| output.cursor_icon = CursorIcon::ResizeVertical);
		return
	}

	#[cfg(target_os = "macos")]
	if response.drag_started() {
		let _ = core_graphics::display::CGDisplay::main().hide_cursor();
	}

	if let Some(origin) = ui.input(|input| input.pointer.press_origin()) {
		let offset = wrap_cursor(ui, own_handle, ui.ctx().pixels_per_point(), origin);

		if offset != Vec2::ZERO {
			state.queued_mouse_offset = Some(offset);
		}
	}

	ui.ctx().output_mut(|output| output.cursor_icon = CursorIcon::None);
}


fn sflt_drag_value<Num: egui::emath::Numeric, T: MessageHandler, S: SfltStore>(ui: &mut Ui, state: &mut SfltUiState<T, S>, num: &mut Num, zero_val: Option<&'static str>, min: Num, max: Num) -> Response {
	let mut dragval =
		DragValue::new(num)
			.with_delta_offset(-state.mouse_offset.y)
			.speed(0.1);

	if let Some(zero_val) = zero_val {
		dragval = dragval.custom_formatter(|n, _| {
			if n == 0.0 {
				zero_val.into()
			} else {
				format!("{}", n as i32)
			}
		});
	}

	let mut response = ui.add(dragval);

	if response.dragged() {
		sflt_widget_wrap(&response, ui, state);
	}

	widget_scroll_behaviour(ui, num, &mut response, min.to_f64(), max.to_f64(), Some(1.0));

	if response.drag_stopped() {
		if let Some(own_handle) = state.own_handle && state.capture_mouse {
			jump_cursor(own_handle, response.rect.center(), ui.ctx().pixels_per_point());
			state.did_mouse_jump = true;
		}
	}

	response
}


fn drag_hints(ui: &mut Ui, unclampable: bool) {
	let alt = if cfg!(target_os = "macos") { "[Option]" } else { "[Alt]" };

	let mut string = String::from("[Ctrl] precision editing");

	if unclampable {
		string += "\n";
		string += alt;
		string += " unclamped range";
	}
	
	place_title_color(ui, string, Pos2::new(7.0, 336.0), Color32::DARK_GRAY);
}


fn sflt_knob<T: MessageHandler, S: SfltStore>(ui: &mut Ui, state: &mut SfltUiState<T, S>, param: SfltParam, max: f32, label: &'static str, hint: &mut HintText, store: &impl SfltStore) -> Response {
	let active_channel = state.get_active_channel();
	let active = state.active_drag == Some(ActiveControl::Param(param));
	let mut value = state.state.param(param, active_channel);

	let linked = if let Some(channel) = active_channel {
		store.linkage(channel).is_linked(param)
	} else {
		false
	};

	if active {
		value += state.drag_overflow;
	}

	let mut knob = Knob::new(&mut value, -1.0, max, knob::KnobStyle::Wiper)
		.with_label(label, knob::LabelPosition::Top)
		.with_delta_offset(-state.mouse_offset.y)
		.with_size(35.0);

	if linked {
		knob = knob
				.with_colors(
					HIGHLIGHT_COLOR.blend(Color32::from_white_alpha(50)),
					HIGHLIGHT_COLOR,
					HIGHLIGHT_COLOR.blend(Color32::from_black_alpha(50))
				);
	}

	let mut response = ui.add(knob);

	widget_scroll_behaviour(ui, &mut value, &mut response, -1.0, max as f64, None);

	if response.changed() {
		let value_clamped = value.clamp(-1.0, max);

		if linked {
			state.push_message(SfltUiMessage::ParamChanged(param, value_clamped, None));
		} else {
			state.push_message(SfltUiMessage::ParamChanged(param, value_clamped, active_channel));
		}

		if active {
			state.drag_overflow = value - value_clamped;
		}
	}

	if response.dragged() {
		state.active_drag = Some(ActiveControl::Param(param));
		*hint = HintText::Edit(param);
		sflt_widget_wrap(&response, ui, state);
		drag_hints(ui, false);
	} else if response.hovered() {
		*hint = HintText::ParamHover(param, value)
	}

	if response.drag_stopped() {
		state.drag_overflow = 0.0;
		state.active_drag = None;

		if let Some(own_handle) = state.own_handle && state.capture_mouse {
			jump_cursor(own_handle, response.rect.center(), ui.ctx().pixels_per_point());
			state.did_mouse_jump = true;
		}
	}

	param_multi_context_menu(&response, param, state, store);

	response
}

fn context_clicked(response: &Response) -> bool {
	#[cfg(not(target_os = "macos"))]
	{
		response.secondary_clicked()
	}

	#[cfg(target_os = "macos")]
	{
		response.secondary_clicked() || response.clicked() && response.ctx.input(|i| i.modifiers.ctrl)
	}
}

fn non_context_clicked(response: &Response) -> bool {
	#[cfg(not(target_os = "macos"))]
	{
		response.clicked()
	}

	#[cfg(target_os = "macos")]
	{
		response.clicked() && !response.ctx.input(|i| i.modifiers.ctrl)
	}
}

fn set_modal_open(response: &Response, value: bool) {
	response.ctx.memory_mut(|mem| {
		*mem.data.get_temp_mut_or(response.id.with("modal"), false) = value;
	});
}

fn type_in_modal(
	response: &Response,
	set_value_buf: &mut String,
	just_opened: bool,
) -> bool {
	
	let modal_id = response.id.with("modal");
	let modal_open = response.ctx.memory(|mem| mem.data.get_temp(modal_id)).unwrap_or(false);
	let mut confirmed = false;

	if modal_open {
		Modal::new(modal_id)
			.show(&response.ctx, |ui| {
				ui.allocate_ui_with_layout(Vec2::new(200.0, 0.0), Layout::top_down(Align::LEFT), |ui| {
					ui.add(Label::new(RichText::new("type in value..").weak()).selectable(false));

					let entry = TextEdit::singleline(set_value_buf)
						.font(TextStyle::Heading);

					let entry_response = entry.show(ui).response;

					if just_opened {
						entry_response.request_focus();
					}

					if entry_response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
						confirmed = true;
						set_modal_open(response, false);
						ui.close();
					}

					ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
						if ui.button("cancel").clicked() {
							set_modal_open(response, false);
							ui.close();
						}

						if ui.button("set").clicked() {
							confirmed = true;
							set_modal_open(response, false);
							ui.close();
						}
					});
				});
			});
	}

	confirmed
}

fn base_context_menu<R>(
	response: &Response,
	add_contents: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
	Popup::menu(response)
		.open_memory(if context_clicked(response) {
			Some(SetOpenCommand::Bool(true))
		} else if non_context_clicked(response) {
			// Explicitly close the menu if the widget was clicked
			// Without this, the context menu would stay open if the user clicks the widget
			Some(SetOpenCommand::Bool(false))
		} else {
			None
		})
		.at_pointer_fixed()
		.show(add_contents)
		.map(|i| i.inner)
}

#[allow(dead_code)] // unused in FL version
fn global_context_menu<T: Copy>(
	response: &Response,
	mut set: impl FnMut(T),
	default_val: T,
) {
	base_context_menu(response, |ui| {
		if ui.button("reset").clicked() {
			set(default_val);
		}
	});
}

fn base_multi_context_menu<T: Copy, R>(
	response: &Response,
	mut set: impl FnMut(T, Option<u8>),
	get: impl Fn(Option<u8>) -> T,
	default_val: T,
	active_channel: Option<u8>,
	add_contents: impl FnOnce(&mut Ui) -> R,
	mut set_value_buf: Option<&mut String>,
	from_string: impl Fn(&str) -> T,
	to_string: impl Fn(T) -> String,
) -> Option<R> {
	let mut modal_just_opened = false;

	let r = base_context_menu(response, |ui| {
		let r = ui.scope(add_contents);

		SubMenuButton::new("copy to..")
			.ui(ui, |ui| {
				for i in 0..16 {
					if ui.add_enabled(Some(i) != active_channel, Button::new(format!("ch. {}", i + 1))).clicked() {
						let val = get(active_channel);

						set(val, Some(i));
					}
				}

				ui.separator();

				if ui.button("all ch.").clicked() {
					let val = get(active_channel);

					for i in 0..16 {
						set(val, Some(i));
					}
				}
			});

		ui.separator();

		if let Some(set_value_buf) = &mut set_value_buf {
			if ui.button("type in..").clicked() {
				**set_value_buf = to_string(get(active_channel));
				set_modal_open(response, true);
				modal_just_opened = true;
			}
		}

		if ui.button("reset").clicked() {
			set(default_val, active_channel);
		}
		
		r.inner
	});
	
	
	if let Some(set_value_buf) = &mut set_value_buf {
		if type_in_modal(response, set_value_buf, modal_just_opened) {
			set(from_string(set_value_buf), active_channel);
		}
	}

	r
}


fn multi_context_menu<T: Copy>(
	response: &Response,
	set: impl FnMut(T, Option<u8>),
	get: impl Fn(Option<u8>) -> T,
	default_val: T,
	active_channel: Option<u8>,
	set_value_buf: Option<&mut String>,
	from_string: impl Fn(&str) -> T,
	to_string: impl Fn(T) -> String,
) {
	base_multi_context_menu(
		response,
		set,
		get,
		default_val,
		active_channel,
		|_: &mut Ui| {},
		set_value_buf,
		from_string,
		to_string,
	);
}


fn param_multi_context_menu(
	response: &Response,
	param: SfltParam,
	state: &mut SfltUiState<impl MessageHandler, impl SfltStore>,
	store: &impl SfltStore,
) {
	let active_channel = state.get_active_channel();
	let multi_mode = state.state.multi_timbral();

	let r = base_multi_context_menu(
		response,
		|val, channel| state.connection.push_message(SfltUiMessage::ParamChanged(param, val, channel)),
		|channel| store.param(param, channel),
		-1.0,
		active_channel,
		|ui: &mut Ui| {
			if let Some(channel) = active_channel {
				SubMenuButton::new("linking..").ui(ui, |ui| {
					let linkage = store.linkage(channel);
					let mut linked = linkage.is_linked(param);

					if ui.checkbox(&mut linked, "linked to master").changed() {
						linkage.set_linked(param, linked);
					}

					ui.separator();

					if ui.button("link all channels").clicked() {
						for i in 0..16 {
							store.linkage(i).set_linked(param, true);
						}
					}

					if ui.button("unlink all channels").clicked() {
						for i in 0..16 {
							store.linkage(i).set_linked(param, false);
						}
					}
				});
			}

			if cfg!(feature = "fl") && !multi_mode {
				if ui.button("fl menu..").clicked() {
					return true
				}
			}

			return false
		},
		Some(&mut state.set_value_buf),
		|text| param_value_from_text(param, &text) as f32,
		|value| get_param_value_text(param, value as i32),
	);

	if let Some(r) = r && r {
		state.connection.push_message(SfltUiMessage::ParamMenuOpened(param, state.own_handle.unwrap()));
	}
}


fn sflt_style() -> Style {
	let mut style = egui::Style::default();

	style.visuals.extreme_bg_color = Color32::from_rgb(14, 14, 14);
	style.visuals.panel_fill = Color32::from_rgb(14, 14, 14);
	style.visuals.override_text_color = Some(Color32::WHITE);
	style.visuals.popup_shadow.offset = [0, 0];
	style.visuals.popup_shadow.spread = 2;
	style
}

fn place_title(ui: &Ui, text: String, pos: Pos2) {
	place_title_color(ui, text, pos, Color32::LIGHT_GRAY);
}

fn place_title_color(ui: &Ui, text: String, pos: Pos2, color: Color32) {
	let painter = ui.painter();
	let mut font = FontId::default();
	let bg = ui.style().visuals.extreme_bg_color;

	font.size = 12.0;

	let galley = painter.layout_no_wrap(text.clone(), font, color);
	let mut bgrect = galley.rect.translate(pos.to_vec2());

	*bgrect.top_mut() += 3.0;
	*bgrect.right_mut() += 2.0;

	bgrect = bgrect.expand2(Vec2::new(1.0, 0.0));

	painter.rect_filled(bgrect, 0.0, bg);
	painter.galley(pos + Vec2::new(1.0, 0.0), galley, Color32::WHITE);
}


#[allow(dead_code)]
fn write_hint_text<T: MessageHandler, S: SfltStore>(ui: &mut Ui, state: &SfltUiState<T, S>, hint: &HintText, pos: Pos2) {
	let painter = ui.painter();
	let mut font = FontId::default();

	font.size = 10.0;

	let text = match hint {
		HintText::None => "".into(),

		HintText::ParamHover(param, val) =>
			format!(
				"{}: {}",
				crate::PARAM_NAMES[*param as usize],
				crate::get_param_value_text(*param, *val as i32)
			),

		HintText::Hover(hint) => (*hint).into(),

		HintText::Edit(param) => crate::get_param_value_text(*param, state.state.param(*param, state.get_active_channel()) as i32),

		HintText::Custom(string) => string.clone()
	};

	let galley = painter.layout_no_wrap(text, font, Color32::GRAY);
	painter.galley(pos - Vec2::new(galley.rect.width(), 0.0), galley, Color32::GRAY);
}


#[allow(unused_variables)]
fn wrap_cursor(ui: &Ui, handle: RawWindowHandle, ppp: f32, drag_start: Pos2) -> Vec2 {
	#[cfg(target_os = "windows")]
	unsafe {
		use windows::Win32::{Foundation, UI::WindowsAndMessaging::{GetCursorPos, GetWindowRect, SetCursorPos}};

		let RawWindowHandle::Win32(handle) = handle else {
			panic!()
		};

		let mut rect = Foundation::RECT::default();

		if !GetWindowRect(HWND(handle.hwnd), &mut rect).is_ok() {
			return Vec2::ZERO
		}

		let mut rawpos = Foundation::POINT::default();

		if !GetCursorPos(&mut rawpos).is_ok() {
			return Vec2::ZERO
		}

		let rawpos = Pos2::new(rawpos.x as f32, rawpos.y as f32);

		let targetpos = Pos2::new(
			rect.left as f32 + drag_start.x * ppp,
			rect.top as f32 + drag_start.y * ppp
		);

		let dist = (rawpos - targetpos).length();

		if dist > 5.0 {
			let _ = SetCursorPos(targetpos.x as i32, targetpos.y as i32);

			Vec2::new(
				(targetpos.x - rawpos.x) / ppp,
				(targetpos.y - rawpos.y) / ppp
			)
		} else {
			Vec2::ZERO
		}
	}

	#[cfg(target_os = "macos")]
	{
		use core_graphics::display::CGDisplay;
		let _ = CGDisplay::associate_mouse_and_mouse_cursor_position(false);
		-ui.input(|i| i.pointer.motion().unwrap_or(Vec2::ZERO))
	}

	#[cfg(target_os = "linux")]
	{
		Vec2::ZERO // todo
	}
}


#[allow(unused_variables)]
fn jump_cursor(handle: RawWindowHandle, pos: Pos2, ppp: f32) {
	#[cfg(target_os = "windows")]
	unsafe {
		use windows::Win32::{Foundation, Graphics::Gdi::ClientToScreen, UI::WindowsAndMessaging::SetCursorPos};

		let RawWindowHandle::Win32(handle) = handle else {
			panic!()
		};

		let mut point = Foundation::POINT {
			x: (pos.x * ppp) as i32, 
			y: (pos.y * ppp) as i32,
		};

		let _ = ClientToScreen(HWND(handle.hwnd), &mut point);
		let _ = SetCursorPos(point.x, point.y);
	}

	#[cfg(target_os = "macos")]
	{
		use core_graphics::display::CGDisplay;
		
		let _ = CGDisplay::associate_mouse_and_mouse_cursor_position(true);
		let _ = CGDisplay::main().show_cursor();
	}
}


fn update_sf_list() {
	let mut db = SOUNDFONT_DB.lock().unwrap();

	// clear unloaded soundfonts
	db.retain(|_, sf| sf.strong_count() != 0);

	*LOADED_SF_LIST.write().unwrap() = db.keys().cloned().collect();
}


pub fn fetch_font_data() -> Option<Vec<(String, Arc<FontData>)>> {
	let font_files = find_cjk_fonts()?;

	let mut result = vec![];
	result.reserve(font_files.len());

	for font_file in font_files {
		let font_name = font_file.split('/').last()?.split('.').next()?.to_string();
		let font_file_bytes = std::fs::read(font_file).ok()?;
		let font_data = FontData::from_owned(font_file_bytes);

		result.push((font_name, Arc::new(font_data)));
	}

	Some(result)
}


pub fn configure_fonts(ctx: &Context) -> Option<Arc<Vec<(String, Arc<FontData>)>>> {
	let mut font_def = FontDefinitions::default();
	let fontdata = &mut *FONT_DATA.lock().unwrap();

	let fonts = if let Some(fonts) = fontdata.upgrade() {
		fonts
	} else {
		let loaded_fonts = Arc::new(fetch_font_data().unwrap_or(vec![]));
		*fontdata = Arc::downgrade(&loaded_fonts);
		loaded_fonts
	};

	let font_family = FontFamily::Proportional;

	for (font_name, font_data) in &*fonts {
		font_def.font_data.insert(font_name.clone(), font_data.clone());

		font_def.families.get_mut(&font_family)?.push(font_name.clone());
	}

	ctx.set_fonts(font_def);
	Some(fonts)
}


fn find_cjk_fonts() -> Option<Vec<String>> {
	let mut result = vec![];

	#[cfg(unix)]
	{
		use std::process::Command;
		// linux/macOS command: fc-list
		let output = Command::new("sh").arg("-c").arg("fc-list").output().ok()?;
		let stdout = std::str::from_utf8(&output.stdout).ok()?;
		
		#[cfg(target_os = "macos")]
		{
			let font_line = stdout
				.lines()
				.find(|line| line.contains("Regular") && line.contains("Apple Gothic"))
				.unwrap_or("/System/Library/Fonts/AppleSDGothicNeo.ttc");

			let font_path = font_line.split(':').next()?.trim();
			result.push(font_path.to_string());

			let font_line = stdout
				.lines()
				.find(|line| line.contains("Regular") && line.contains("Hiragino Sans GB"))
				.unwrap_or("/System/Library/Fonts/Hiragino Sans GB.ttc");

			let font_path = font_line.split(':').next()?.trim();
			result.push(font_path.to_string());
		}

		#[cfg(target_os = "linux")]
		{
			let font_line = stdout
				.lines()
				.find(|line| line.contains("Regular") && line.contains("CJK"))
				.unwrap_or("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc");

			let font_path = font_line.split(':').next()?.trim();
			result.push(font_path.to_string());
		}
	}

	#[cfg(windows)]
	{
		let font_file = {
			// MS YaHei covers most of it
			let mut font_path = PathBuf::from(std::env::var("SystemRoot").ok()?);
			font_path.push("Fonts");
			font_path.push("msyh.ttc");
			font_path.to_str()?.to_string().replace("\\", "/")
		};

		result.push(font_file);

		let font_file = {
			// Malgun Gothic for Korean
			let mut font_path = PathBuf::from(std::env::var("SystemRoot").ok()?);
			font_path.push("Fonts");
			font_path.push("malgun.ttf");
			font_path.to_str()?.to_string().replace("\\", "/")
		};

		result.push(font_file);
	}

	Some(result)
}


pub fn egui_window<T: MessageHandler + 'static, S: SfltStore + 'static>(
	state: Arc<Mutex<SfltUiState<T, S>>>,
	window_options: WindowOpenOptions,
	parent: RawWindowHandle
) -> WindowHandle {
	egui_baseview::EguiWindow::open_parented(
		// hack to deal with HasRawWindowHandle. i have no idea what i'm doing
		&FuckYou(parent),

		window_options,

		GraphicsConfig {
			dithering: false,

			#[cfg(feature = "glow")]
			shader_version: None,

			#[cfg(feature = "wgpu")]
			wgpu_options: WgpuConfiguration {
				wgpu_setup: WgpuSetup::CreateNew(WgpuSetupCreateNew {
					instance_descriptor: InstanceDescriptor {
						backends: Backends::METAL | Backends::VULKAN | Backends::DX12,
					
						..Default::default()
					},
					
					power_preference: PowerPreference::LowPower,

					..Default::default()
				}),
				..Default::default()
			},
		},

		state,

		|ctx: &Context, _queue: &mut _, state: &mut Arc<Mutex<SfltUiState<T, S>>>| egui_build(ctx, &mut *state.lock().unwrap()),

		|ctx: &Context, _queue: &mut _, state: &mut Arc<Mutex<SfltUiState<T, S>>>| egui_update(ctx, &mut *state.lock().unwrap())
	)
}


pub fn egui_build<T: MessageHandler + 'static, S: SfltStore + 'static>(ctx: &Context, state: &mut SfltUiState<T, S>) {
	egui_extras::install_image_loaders(ctx);
	state.fonts = configure_fonts(ctx);
}


pub fn egui_update<T: MessageHandler + 'static, S: SfltStore + 'static>(ctx: &Context, state: &mut SfltUiState<T, S>) {

	egui::CentralPanel::default().show(&ctx, |ui| {
		state.try_flush_messages();
		state.process_engine_messages();

		let page_dirty = state.page_dirty;

		state.page_dirty = false;
		// nasty
		state.capture_mouse = CONFIG.read().unwrap().capture_mouse;

		if let Some(offset) = state.queued_mouse_offset.take() {
			state.mouse_offset = offset;
		} else {
			state.mouse_offset = Vec2::ZERO;
		}

		ctx.set_style(sflt_style());

		let mut hint = HintText::None;
		let mut preview_note = None;

		let active_channel = state.get_active_channel();

		if active_channel != state.prev_active_channel {
			state.update_preset_scroll_list();
			state.prev_active_channel = active_channel;
		}

		let store = state.state.clone();
		let dummy = store.multi_timbral() && active_channel.is_none();

		ui.input(|input| {
			if let Some(file) = input.raw.dropped_files.first()
				&& let Some(path) = file.path.as_ref()
				&& let Some(ext) = path.extension()
			{
				match ext.to_string_lossy().to_lowercase().as_str() {
					"sf2" | "sf3" => {
						state.loading = true;
						state.load_error = None;
						state.push_message(SfltUiMessage::SoundFontChanged(path.clone(), active_channel));
					}

					"txt" => {
						state.push_message(SfltUiMessage::LoadAuxFile(AuxFileKind::KeyNames, path.clone(), active_channel));
					}

					"scl" => {
						state.push_message(SfltUiMessage::LoadAuxFile(AuxFileKind::Tuning, path.clone(), active_channel));
					}

					"kbm" => {
						state.push_message(SfltUiMessage::LoadAuxFile(AuxFileKind::Mapping, path.clone(), active_channel));
					}

					_ => {}
				}
			}
		});

		ui.add_enabled_ui(!state.loading, |ui| {
			match &mut state.page {

				SfltUiPage::Main => {
					// FILE
					sflt_frame(ctx).show(ui, |ui| {
						ui.with_layout(Layout::right_to_left(egui::Align::TOP), |ui| {
							let menu = ui.button("▼");

							Popup::menu(&menu).align(RectAlign::BOTTOM_END).gap(2.0).show(|ui| {
								{
									let sflocator = ui.button("locate..");

									if sflocator.clicked() {
										state.push_message(SfltUiMessage::SoundFontLocator(active_channel));
									}

									if sflocator.hovered() {
										hint = HintText::Hover("Use native file browser");
									}
								}

								{
									let clearsf = ui.add_enabled(
										state.state.soundfont(active_channel).is_some(),
										Button::new("clear")
									);

									if clearsf.clicked() {
										state.push_message(SfltUiMessage::SoundFontCleared(active_channel));
									}

									if clearsf.hovered() {
										hint = HintText::Hover("Clear active SoundFont");
									}
								}

								ui.add_sized(Vec2::new(150.0, 0.0), Separator::default());

								if cfg!(feature = "fl") && !dummy {
									SubMenuButton::new("key names..")
										.ui(ui, |ui| {
											let keylocator = ui.button("locate.. (.txt)");

											if keylocator.clicked() {
												state.push_message(SfltUiMessage::LocateAuxFile(AuxFileKind::KeyNames, active_channel));
											}

											let keyclear = ui.button("clear");

											if keyclear.clicked() {
												state.push_message(SfltUiMessage::ClearAuxFile(AuxFileKind::KeyNames, active_channel));
											}
										});
								}

								SubMenuButton::new("tuning..")
									.ui(ui, |ui| {
										let locator = ui.button("load tuning.. (.scl)");

										if locator.clicked() {
											state.push_message(SfltUiMessage::LocateAuxFile(AuxFileKind::Tuning, active_channel));
										}

										let clear = ui.button("clear tuning");

										if clear.clicked() {
											state.push_message(SfltUiMessage::ClearAuxFile(AuxFileKind::Tuning, active_channel));
										}

										let locator = ui.button("load mapping.. (.kbm)");

										if locator.clicked() {
											state.push_message(SfltUiMessage::LocateAuxFile(AuxFileKind::Mapping, active_channel));
										}

										let clear = ui.button("clear mapping");

										if clear.clicked() {
											state.push_message(SfltUiMessage::ClearAuxFile(AuxFileKind::Mapping, active_channel));
										}

										let mut zone_remapping = state.state.zone_remapping(active_channel);
										let mut smart_pitching = state.state.smart_pitching(active_channel);

										if ui.checkbox(&mut zone_remapping, "zone remapping").changed() {
											state.push_message(SfltUiMessage::ZoneRemappingChanged(zone_remapping, active_channel));

										}

										if ui.checkbox(&mut smart_pitching, "smart bending").changed() {
											state.push_message(SfltUiMessage::SmartPitchingChanged(smart_pitching, active_channel));
										}
									});

								ui.add_sized(Vec2::new(150.0, 0.0), Separator::default());

								for sf in &*LOADED_SF_LIST.read().unwrap() {
									let Some(filename) = sf.file_name() else {
										continue
									};

									if ui.add(Button::new(filename.to_string_lossy()).wrap_mode(TextWrapMode::Truncate)).clicked() {
										state.push_message(SfltUiMessage::SoundFontChanged(sf.clone(), active_channel));
										state.load_error = None;
										ui.close();
									}
								}

							});

							if menu.clicked() {
								update_sf_list();
							}

							let mut sfpath = None;
							let mut sfname = "(No SoundFont loaded)".into();
							let mut using_fallback = false;
							let mut load_progress = 0;

							let sf = &*state.state.soundfont(active_channel);

							if let Some(msg) = state.load_error {
								sfname = format!("ERROR: ({msg})").into();
							}
							else if state.loading {
								load_progress = state.load_progress.load(Ordering::Relaxed);

								if load_progress & SM24_FLAG != 0 {
									load_progress = load_progress & !SM24_FLAG;
									sfname = format!("(Loading 24-bit... {:.1}%)", load_progress as f32 / 10.0).into();
								} else if load_progress & OGG_FLAG != 0 {
									load_progress = load_progress & !OGG_FLAG;
									sfname = format!("(Decompressing samples... {:.1}%)", load_progress as f32 / 10.0).into();
								} else {
									sfname = format!("(Loading... {:.1}%)", load_progress as f32 / 10.0).into();
								}
							}
							else if let Some(sf) = &sf {
								sfpath = Some(sf.path.file_name().unwrap().to_string_lossy());

								if let Some(name) = &sf.name {
									sfname = name.into();
								} else {
									sfname = sfpath.as_ref().unwrap().clone()
								}
							} else if active_channel.is_some() {
								let mut parent_name = "(None)".into();

								let parent_sf = state.state.soundfont(None);

								if let Some(sf) = &*parent_sf {
									sfpath = Some(sf.path.file_name().unwrap().to_string_lossy().into_owned().into());

									if let Some(name) = &sf.name {
										parent_name = name.into();
									} else {
										parent_name = sfpath.as_ref().unwrap().clone()
									}
								}

								using_fallback = true;
								sfname = format!("Master: {}", parent_name).into();
							}

							// this is silly
							ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 0.0), Layout::top_down_justified(Align::LEFT), |ui| {
								if state.load_error.is_some() {
									ui.style_mut().visuals.override_text_color = Some(
										Color32::LIGHT_RED
									);
								}
								else if using_fallback {
									ui.style_mut().visuals.override_text_color = Some(
										Color32::GRAY
									);
								}

								let mut filebrowser = ui.add(Button::new(sfname).wrap_mode(TextWrapMode::Truncate));

								if let Some(sfpath) = sfpath {
									filebrowser = filebrowser.on_hover_text(sfpath);
								}

								if filebrowser.hovered() {
									hint = HintText::Hover("File browser");
								}

								if filebrowser.clicked() {
									state.page = SfltUiPage::FileBrowser {
										entries: vec![],
										filtered: vec![]
									};
									state.page_dirty = true;
									state.load_error = None;
								}

								if state.loading {
									let rect = filebrowser.rect.split_left_right_at_fraction(load_progress as f32 / 1000.0).0;

									ui.painter().rect_filled(
										rect,
										ui.style().visuals.widgets.active.corner_radius,
										Color32::from_white_alpha(50)
									);
								}
							});
						});

						ui.separator();

						ui.with_layout(Layout::left_to_right(egui::Align::TOP), |ui| {
							let mut preset_linked = false;

							let patchbank = [
								(SfltParam::PatchNumber, 127),
								(SfltParam::BankNumber, 128),
							];

							for (param, max) in patchbank {
								let linked = if let Some(channel) = active_channel {
									store.linkage(channel).is_linked(param)
								} else {
									false
								};

								let mut value = state.state.param(param, active_channel) as u8;
								let drag = ui.scope(|ui| {
									if linked {
										ui.style_mut().visuals.widgets.active.weak_bg_fill = HIGHLIGHT_COLOR;
										ui.style_mut().visuals.widgets.inactive.weak_bg_fill = HIGHLIGHT_COLOR;
										ui.style_mut().visuals.widgets.hovered.weak_bg_fill = HIGHLIGHT_COLOR;
										preset_linked = true;
									}

									sflt_drag_value(ui, state, &mut value, None, 0, max)
								}).inner;

								if drag.changed() {
									value = value.clamp(0, max);

									if linked {
										state.push_message(SfltUiMessage::ParamChanged(param, value as f32, None));
									} else {
										state.push_message(SfltUiMessage::ParamChanged(param, value as f32, active_channel));
									}
								}

								if !drag.dragged() && drag.hovered() {
									hint = HintText::ParamHover(param, value as f32);
								}

								param_multi_context_menu(&drag, param, state, &store);
							}

							let mut presetname = "(None)";
							let mut preset_idx = None;
							let mut new_preset = None;

							{
								let mut sf = state.state.soundfont(active_channel);

								if sf.is_some() {
									// have to do this here because borrow issues
									let Some(sf) = &*sf else { unreachable!() };
									let preset = (
										state.state.param(SfltParam::BankNumber, active_channel) as u8,
										state.state.param(SfltParam::PatchNumber, active_channel) as u8
									);

									if let Some(idx) = sf.get_preset_idx(preset.0, preset.1) {
										let preset = sf.presets.get(idx).unwrap();
										presetname = &preset.name;
										preset_idx = Some(idx);
									}
								} else if active_channel.is_some() {
									// fetch preset from parent soundfont instead
									sf = state.state.soundfont(None);

									if let Some(sf) = &*sf {
										let preset = (
											state.state.param(SfltParam::BankNumber, active_channel) as u8,
											state.state.param(SfltParam::PatchNumber, active_channel) as u8
										);

										if let Some(idx) = sf.get_preset_idx(preset.0, preset.1) {
											let preset = sf.presets.get(idx).unwrap();
											presetname = &preset.name;
											preset_idx = Some(idx);
										}
									}
								}


								ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 0.0), Layout::top_down_justified(Align::LEFT), |ui| {
									if preset_idx.is_none() {
										ui.style_mut().visuals.override_text_color = Some(
											Color32::GRAY
										);
									}

									let presetbrowser = ui.add_enabled(!preset_linked, Button::new(presetname).wrap_mode(TextWrapMode::Truncate));

									if presetbrowser.hovered() {
										hint = HintText::Hover("Preset browser");

										if let Some(sf) = &*sf && let Some(preset_idx) = preset_idx {
											let delta = get_scroll_delta(ui) as f64;

											if delta != 0.0 {
												let mut current_scroll_idx =
													state
														.preset_scroll_list
														.iter()
														.position(|i| *i == preset_idx)
														.unwrap_or(0);

												if delta > 0.0 {
													if current_scroll_idx == 0 {
														current_scroll_idx = sf.presets.len() - 1;
													} else {
														current_scroll_idx -= 1;
													}
												} else {
													current_scroll_idx = (current_scroll_idx + 1) % sf.presets.len();
												}

												current_scroll_idx = current_scroll_idx % sf.presets.len();

												let new_idx = state.preset_scroll_list.get(current_scroll_idx).copied().unwrap_or(0);

												new_preset = Some((
													sf.presets[new_idx].bank as u8,
													sf.presets[new_idx].preset as u8
												));
											}
										}
									}

									if presetbrowser.clicked() {
										if state.state.soundfont(active_channel).is_some() || state.state.soundfont(None).is_some() {
											state.page = SfltUiPage::PresetBrowser { filtered: vec![] };
											state.page_dirty = true;
											state.load_error = None;
										}
									}
								});
							}

							if let Some(new_preset) = new_preset {
								state.push_message(SfltUiMessage::ParamChanged(SfltParam::BankNumber, new_preset.0 as f32, active_channel));
								state.push_message(SfltUiMessage::ParamChanged(SfltParam::PatchNumber, new_preset.1 as f32, active_channel));
							}
						});
					});

					ui.add_space(3.0);

					const TRACK_MIN: i32 = if cfg!(feature = "fl") { -125 } else { 0 };
					const TRACK_MAX: i32 = if cfg!(feature = "fl") { 125 } else { 16 };

					ui.columns(2, |columns| {
						columns[0].with_layout(Layout::top_down_justified(Align::LEFT), |ui| {
							// REVERB
							let centered = Layout::top_down(egui::Align::Center).with_main_align(egui::Align::Center);

							sflt_frame(ctx).show(ui, |ui| {
								ui.columns(2, |columns| {

									columns[0].with_layout(centered, |ui| {
										ui.add_space(8.0);

										ui.add_enabled_ui(!dummy, |ui| {
											let mut send = state.state.reverb_out(active_channel);
											let response = sflt_drag_value(ui, state, &mut send, Some("---"), TRACK_MIN, TRACK_MAX);

											if response.hovered() {
												if cfg!(feature = "fl") {
													hint = HintText::Hover("Send reverb to effect track number");
												} else {
													hint = HintText::Hover("Send reverb to auxiliary output");
												}
											}

											if response.changed() {
												send = send.clamp(TRACK_MIN, TRACK_MAX);
												state.push_message(SfltUiMessage::FxTracksChanged(send, state.state.chorus_out(active_channel), active_channel));
											}

											multi_context_menu(
												&response,
												|val, channel| state.connection.push_message(SfltUiMessage::FxTracksChanged(val, state.state.chorus_out(channel), channel)),
												|channel| store.reverb_out(channel),
												0,
												active_channel,
												Some(&mut state.set_value_buf),
												|text| value_from_text(text, TRACK_MIN, TRACK_MAX, 0),
												|value| (value as i32).to_string(),
											);

											let mode = ComboBox::from_id_salt("reverbmode")
												.selected_text(state.state.reverb(active_channel).to_string())
												.width(ui.available_width())
												.show_ui(ui, |ui| {
													let mut reverb = state.state.reverb(active_channel);

													for mode in [ReverbMode::Disabled, ReverbMode::Dry] {
														if ui.selectable_value(&mut reverb, mode, mode.to_string()).clicked() {
															state.push_message(SfltUiMessage::FxChanged(reverb, state.state.chorus(active_channel), active_channel));
														}
													}
												});

											if mode.response.hovered() {
												hint = HintText::Hover("Reverb mode");
											}

											multi_context_menu(
												&mode.response,
												|val, channel| state.connection.push_message(SfltUiMessage::FxChanged(val, state.state.chorus(channel), channel)),
												|channel| store.reverb(channel),
												ReverbMode::Disabled,
												active_channel,
												None,
												|_| Default::default(),
												|value| (value as i32).to_string(),
											);
										});
									});

									columns[1].with_layout(centered, |ui| {
										sflt_knob(ui, state, SfltParam::ReverbSend, 200.0, "LVL", &mut hint, &store);
									});
								});
							});

							ui.add_space(3.0);

							// ADSR
							sflt_frame(ctx).show(ui, |ui| {
								let adsr = [SfltParam::Env2Attack, SfltParam::Env2Decay, SfltParam::Env2Sustain, SfltParam::Env2Release];
								let text = ["A", "D", "S", "R"];

								ui.style_mut().spacing.slider_width = 110.0;

								let alt = ui.input(|i| i.modifiers.matches_any(Modifiers::ALT));

								ui.columns(4, |columns| {
									let handle = egui::style::HandleShape::Rect { aspect_ratio: 0.8 };

									for (i, param) in adsr.into_iter().enumerate() {
										columns[i].with_layout(Layout::top_down(Align::Center), |ui| {
											let mut paramval = state.state.param(param, active_channel);
											let range_max = if param == SfltParam::Env2Sustain { 127.0 } else { 5940.0 };
											let active = state.active_drag == Some(ActiveControl::Param(param));

											let max = if paramval > 5940.0 {
												paramval
											} else {
												range_max
											};

											let linked = if let Some(channel) = active_channel {
												store.linkage(channel).is_linked(param)
											} else {
												false
											};

											if active {
												paramval += state.drag_overflow;
											}

											ui.add_space(3.0);

											let mut min = ui.next_widget_position();

											min.x -= 8.5;

											let mut response = ui.scope(|ui| {
												if linked {
													ui.style_mut().visuals.widgets.inactive.bg_fill = HIGHLIGHT_COLOR;
												}

												let slider = Slider::new(&mut paramval, -1.0..=range_max)
													.show_value(false)
													.vertical()
													.clamping(slider::SliderClamping::Never)
													.handle_shape(handle)
													.with_delta_offset(-state.mouse_offset.y);

												ui.put(Rect { min, max: min }, slider)
											}).inner;

											widget_scroll_behaviour(ui, &mut paramval, &mut response, -1.0, range_max as f64, None);

											if response.changed() {
												let value_clamped;

												if alt && param != SfltParam::Env2Sustain {
													value_clamped = paramval.max(-1.0);
												} else {
													value_clamped = paramval.clamp(-1.0, max);
												}

												if linked {
													state.push_message(SfltUiMessage::ParamChanged(param, value_clamped, None));
												} else {
													state.push_message(SfltUiMessage::ParamChanged(param, value_clamped, active_channel));
												}

												if active {
													state.drag_overflow = paramval - value_clamped;
												}
											}

											if response.dragged() {
												state.active_drag = Some(ActiveControl::Param(param));
												hint = HintText::Edit(param);
												sflt_widget_wrap(&response, ui, state);
												drag_hints(ui, param != SfltParam::Env2Sustain);
											} else if response.hovered() {
												hint = HintText::ParamHover(param, paramval);
											}

											if response.drag_stopped() {
												state.drag_overflow = 0.0;
												state.active_drag = None;
												paramval = paramval.clamp(0.0, max);

												if let Some(own_handle) = state.own_handle && state.capture_mouse {
													let mut pos = response.rect.center();

													pos.y -= ((paramval / max) - 0.5) * 100.0;

													jump_cursor(own_handle, pos, ctx.pixels_per_point());
													state.did_mouse_jump = true;
												}
											}

											param_multi_context_menu(
												&response,
												param,
												state,
												&store
											);

											ui.add_space(2.0);

											ui.add(Label::new(text[i]).selectable(false));
										});
									}
								});
							});

							ui.add_space(3.0);

							ui.add_enabled_ui(!dummy, |ui| {
								ui.with_layout(Layout::left_to_right(Align::TOP), |ui| {
									let combo = ComboBox::from_id_salt("resampling")
										.selected_text(state.state.resampling(active_channel).to_string())
										.width(72.0)
										.popup_align(RectAlign::TOP_START)
										.show_ui(ui, |ui| {
											let mut resampling = state.state.resampling(active_channel);

											let modes = [
												(ResampleMethod::Gaussian, "Blurry algorithm"),
												(ResampleMethod::Hermite,  "Smooth algorithm"),
												(ResampleMethod::Linear,   "Default algorithm"),
												(ResampleMethod::Nearest,  "Crunchy algorithm"),
											];

											for (mode, desc) in modes {
												let button = ui.selectable_value(&mut resampling, mode, mode.to_string());

												if button.clicked() {
													state.push_message(SfltUiMessage::ResamplingChanged(resampling, active_channel));
												}

												if button.hovered() {
													hint = HintText::Hover(desc);
												}
											}
										});

									if combo.response.hovered() {
										hint = HintText::Hover("Resampling method");
									}

									multi_context_menu(
										&combo.response,
										|val, channel| state.connection.push_message(SfltUiMessage::ResamplingChanged(val, channel)),
										|channel| store.resampling(channel),
										ResampleMethod::Linear,
										active_channel,
										None,
										|_| ResampleMethod::Linear,
										|value| (value as i32).to_string(),
									);

									let mut bypass = state.state.filter_bypass(active_channel);

									let checkbox = ui.add(Checkbox::new(&mut bypass, "filter bypass"));

									if checkbox.changed() {
										state.push_message(SfltUiMessage::FilterBypassChanged(bypass, active_channel));
									}
									if checkbox.hovered() {
										hint = HintText::Hover("Disable built-in lowpass");
									}

									multi_context_menu(
										&checkbox,
										|val, channel| state.connection.push_message(SfltUiMessage::FilterBypassChanged(val, channel)),
										|channel| store.filter_bypass(channel),
										false,
										active_channel,
										None,
										|_| Default::default(),
										|value| (value as i32).to_string(),
									);
								});
							});

							ui.add_space(34.0);

							ui.with_layout(Layout::left_to_right(Align::TOP), |ui| {
								if ui.button("settings").clicked() {
									state.page = SfltUiPage::Settings;
									state.page_dirty = true;
									state.load_error = None;
								}

								let mut multi = store.multi_timbral();
								let mut channel = state.active_channel;

								let checkbox = ui.add(Checkbox::new(&mut multi, "multi"));

								if checkbox.changed() {
									state.push_message(SfltUiMessage::MultiTimbralChanged(multi));
								} else if checkbox.hovered() {
									hint = HintText::Hover("Toggle multitimbral mode");
								}

								if multi {
									let chanval = sflt_drag_value(ui, state, &mut channel, Some("---"), 0, 16);

									channel = channel.clamp(0, 16);

									if chanval.changed() {
										state.active_channel = channel;
									} else if !chanval.dragged() && chanval.hovered() {
										hint = HintText::Hover("Selected multitimbral channel")
									}
								}
							});
						});

						columns[1].with_layout(Layout::top_down_justified(Align::Center), |ui| {
							// CHORUS
							let centered = Layout::top_down(egui::Align::Center).with_main_align(egui::Align::Center);

							sflt_frame(ctx).show(ui, |ui| {
								ui.columns(2, |columns| {
									columns[0].with_layout(centered, |ui| {
										ui.add_space(8.0);

										ui.add_enabled_ui(!dummy, |ui| {
											let mut send = state.state.chorus_out(active_channel);
											let response = sflt_drag_value(ui, state, &mut send, Some("---"), TRACK_MIN, TRACK_MAX);

											if response.hovered() {
												if cfg!(feature = "fl") {
													hint = HintText::Hover("Send chorus to effect track number");
												} else {
													hint = HintText::Hover("Send chorus to auxiliary output");
												}
											}

											if response.changed() {
												send = send.clamp(TRACK_MIN, TRACK_MAX);
												state.push_message(SfltUiMessage::FxTracksChanged(state.state.reverb_out(active_channel), send, active_channel));
											}

											multi_context_menu(
												&response,
												|val, channel| state.connection.push_message(SfltUiMessage::FxTracksChanged(state.state.reverb_out(channel), val, channel)),
												|channel| store.chorus_out(channel),
												0,
												active_channel,
												Some(&mut state.set_value_buf),
												|text| value_from_text(text, TRACK_MIN, TRACK_MAX, 0),
												|value| (value as i32).to_string(),
											);

											let mode = ComboBox::from_id_salt("chorusmode")
												.selected_text(state.state.chorus(active_channel).to_string())
												.width(ui.available_width())
												.show_ui(ui, |ui| {
													let mut chorus = state.state.chorus(active_channel);

													for mode in [ChorusMode::Disabled, ChorusMode::Dry] {
														if ui.selectable_value(&mut chorus, mode, mode.to_string()).clicked() {
															state.push_message(SfltUiMessage::FxChanged(state.state.reverb(active_channel), chorus, active_channel));
														}
													}
												});

											if mode.response.hovered() {
												hint = HintText::Hover("Chorus mode");
											}

											multi_context_menu(
												&mode.response,
												|val, channel| state.connection.push_message(SfltUiMessage::FxChanged(state.state.reverb(channel), val, channel)),
												|channel| store.chorus(channel),
												ChorusMode::Disabled,
												active_channel,
												None,
												|_| Default::default(),
												|value| (value as i32).to_string(),
											);
										});
									});

									columns[1].with_layout(centered, |ui| {
										//ui.add_enabled_ui(state.state.chorus(active_channel) != ChorusMode::Disabled, |ui| {
											sflt_knob(ui, state, SfltParam::ChorusSend, 200.0, "LVL", &mut hint, &store);
										//});
									});
								});
							});

							ui.add_space(3.0);

							// LFO
							sflt_frame(ctx).show(ui, |ui| {
								ui.columns(3, |columns| {
									columns[0].with_layout(centered, |ui| {
										sflt_knob(ui, state, SfltParam::Lfo2Delay, 5900.0, "DEL", &mut hint, &store);
									});

									columns[1].with_layout(centered, |ui| {
										sflt_knob(ui, state, SfltParam::Lfo2Amount, 256.0, "AMT", &mut hint, &store);
									});

									columns[2].with_layout(centered, |ui| {
										sflt_knob(ui, state, SfltParam::Lfo2Speed, 127.0, "SPD", &mut hint, &store);
									});
								});
							});

							ui.add_space(3.0);

							// MISC
							sflt_frame(ctx).show(ui, |ui| {
								ui.columns(3, |columns| {
									columns[0].with_layout(centered, |ui| {
										ui.add_space(8.0);

										ui.add(Label::new(RichText::new("SEND").size(12.0)).selectable(false));

										ui.add_enabled_ui(!dummy, |ui| {
											let mut send = state.state.main_out(active_channel);
											let mut response = sflt_drag_value(ui, state, &mut send, Some("---"), TRACK_MIN, TRACK_MAX);

											if response.hovered() {
												if cfg!(feature = "fl") {
													hint = HintText::Hover("Send audio to track number");
												} else {
													hint = HintText::Hover("Send audio to auxiliary output");
												}
											}

											if response.changed() {
												send = send.clamp(TRACK_MIN, TRACK_MAX);
												state.push_message(SfltUiMessage::MainOutChanged(send, active_channel));
											}

											multi_context_menu(
												&mut response,
												|val, channel| state.connection.push_message(SfltUiMessage::MainOutChanged(val, channel)),
												|channel| store.main_out(channel),
												0,
												active_channel,
												Some(&mut state.set_value_buf),
												|text| value_from_text(text, TRACK_MIN, TRACK_MAX, 0),
												|value| (value as i32).to_string(),
											);
										});
									});

									columns[1].with_layout(centered, |ui| {
										sflt_knob(ui, state, SfltParam::InitialFilterFc, 127.0, "CUT", &mut hint, &store);
									});

									columns[2].with_layout(centered, |ui| {
										sflt_knob(ui, state, SfltParam::Modulation, 127.0, "MOD", &mut hint, &store);
									});
								});
							});

							ui.add_space(4.0);

							let pos = ui.cursor().left_top();
							let size = Vec2::new(ui.available_width(), ui.available_height());
							let offset = Vec2::new(10.0, 16.0);

							let logorect = ui.allocate_rect(Rect::from_min_size(pos + offset, size - offset), Sense::click());

							if logorect.clicked() {
								state.page = SfltUiPage::About;
								state.page_dirty = true;
								state.load_error = None;
							}

							if logorect.hovered() {
								hint = HintText::Hover("About SFLT");
							}

							ui.put(
								Rect::from_min_size(pos, size),
								state
									.logo
									.clone()
									.tint(if logorect.hovered() { Color32::WHITE } else { Color32::DARK_GRAY })
							);
						});
					});

					place_title(ui, "PATCH".into(), Pos2::new(17.0, 2.0));
					place_title(ui, "REVERB".into(), Pos2::new(17.0, 74.0));
					place_title(ui, "CHORUS".into(), Pos2::new(198.0, 74.0));
					place_title(ui, "ENVELOPE 2".into(), Pos2::new(17.0, 152.0));
					place_title(ui, "LFO 2".into(), Pos2::new(198.0, 152.0));
					place_title(ui, "MISC".into(), Pos2::new(198.0, 230.0));
					place_title(ui, VERSION.into(), Pos2::new(329.0, 376.0));

					#[cfg(not(feature = "fl"))]
					{
						write_hint_text(ui, state, &hint, Pos2::new(361.0, 316.0));
					}

					// prevent taking keyboard focus on this screen
					ctx.output_mut(|output| {
						output.ime = None;
						output.events.clear();
					});
				}



				SfltUiPage::FileBrowser { .. } | SfltUiPage::PresetBrowser { .. } => {
					ui.with_layout(Layout::left_to_right(egui::Align::TOP), |ui| {
						if ui.button("back").clicked() {
							state.page = SfltUiPage::Main;
							state.page_dirty = true;
						}

						ui.style_mut().visuals.extreme_bg_color = Color32::BLACK;

						let text_edit = ui.add_sized(
							Vec2::new(ui.available_width(), 0.0),
							TextEdit::singleline(&mut state.text_edit_buf)
								.hint_text("Filter...")
						);

						if page_dirty {
							state.text_edit_buf.clear();
							state.refresh_browser_list();
							state.update_browser_filter();
							text_edit.request_focus();
						}

						if text_edit.changed() {
							state.update_browser_filter();
						}
					});

					ui.separator();

					let paint_bg = |ui: &mut egui::Ui| {
						let item_spacing = ui.spacing().item_spacing;
						let bg = Color32::from_white_alpha(25);
						let gapless_rect = ui.max_rect().expand2(0.5 * item_spacing);
						ui.painter().rect_filled(gapless_rect, 0.0, bg);
					};

					if let SfltUiPage::PresetBrowser { filtered, .. } = &state.page {
						let mut new_preset = None;
						let mut sort_order = state.preset_sort_order;
						let mut sort_ascending = state.preset_sort_ascending;

						if state.state.soundfont(active_channel).is_none() && state.state.soundfont(None).is_none() {
							state.page = SfltUiPage::Main;
							state.page_dirty = true;
							return;
						}

						let labels = [
							["Name", "Name⬆", "Name⬇"],
							["Patch", "Patch⬆", "Patch⬇"],
							["Bank", "Bank⬆", "Bank⬇"]
						];

						let columns = [PresetSortOrder::Alphabetical, PresetSortOrder::Preset, PresetSortOrder::Bank];
						
						#[cfg(not(target_os = "macos"))]
						let secondary_down = ui.input(|i| i.pointer.secondary_down() || i.pointer.secondary_released());

						#[cfg(target_os = "macos")]
						let secondary_down = ui.input(|i| i.modifiers.ctrl && (i.pointer.primary_down() || i.pointer.primary_released()));

						TableBuilder::new(ui)
							.column(Column::remainder())
							.column(Column::exact(45.0))
							.column(Column::exact(45.0))
							.sense(Sense::click())
							.drag_to_scroll(!secondary_down)
							.striped(true)
							.header(15.0, |mut header| {
								for i in 0..columns.len() {
									let response = header.col(|ui| {
										let label;

										if state.preset_sort_order == columns[i] {
											if state.preset_sort_ascending {
												label = labels[i][1];
											} else {
												label = labels[i][2];
											}
										} else {
											label = labels[i][0];
										}

										ui.add(Label::new(label).selectable(false));
									});

									if response.1.clicked() {
										if state.preset_sort_order == columns[i] {
											sort_ascending = !sort_ascending;
										} else {
											sort_order = columns[i];
											sort_ascending = true;
										}
									}
								}
							})
							.body(|body| {
								body.rows(15.0, filtered.len(), |mut row| {
									let mut sf = state.state.soundfont(active_channel);

									if sf.is_none() && active_channel.is_some() {
										sf = state.state.soundfont(None);
									}

									let preset = filtered[row.index()];
									let preset = &sf.as_ref().unwrap().presets[preset];

									let columns: [Cow<'_, str>; 3] = [
										preset.name.as_str().into(),
										preset.preset.to_string().into(),
										preset.bank.to_string().into(),
									];

									let active_preset = (
										state.state.param(SfltParam::BankNumber, active_channel) as u8,
										state.state.param(SfltParam::PatchNumber, active_channel) as u8
									);

									if active_preset == (preset.bank as u8, preset.preset as u8) {
										row.set_selected(true);
									}

									for col in columns {
										let response = row.col(|ui| {
											ui.add(Label::new(col).selectable(false).truncate());
										});

										if response.1.clicked() && !secondary_down {
											new_preset = Some((preset.bank as u8, preset.preset as u8));
										}

										if response.1.contains_pointer() && secondary_down {
											preview_note = Some(filtered[row.index()]);
										}
									}

								});
							});

						if sort_order != state.preset_sort_order || sort_ascending != state.preset_sort_ascending {
							state.preset_sort_order = sort_order;
							state.preset_sort_ascending = sort_ascending;
							state.update_browser_filter();
						}

						if let Some(new_preset) = new_preset {
							state.push_message(SfltUiMessage::ParamChanged(SfltParam::BankNumber, new_preset.0 as f32, active_channel));
							state.push_message(SfltUiMessage::ParamChanged(SfltParam::PatchNumber, new_preset.1 as f32, active_channel));

							if !ui.input(|input| input.modifiers.shift) {
								state.page = SfltUiPage::Main;
								state.page_dirty = true;
							}
						}
					}

					else if let SfltUiPage::FileBrowser { filtered, .. } = &mut state.page {
						let path = &mut state.current_path;
						let mut path_dirty = false;
						let mut new_sf = None;
						let mut sort_ascending = state.file_sort_ascending;

						ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
							ui.add_space(14.0);

							ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
								TableBuilder::new(ui)
									.column(Column::remainder())
									.sense(Sense::click())
									.striped(true)
									.header(15.0, |mut header| {
										if header.col(|ui| {
											if state.file_sort_ascending {
												ui.add(Label::new("Name⬆").selectable(false));
											} else {
												ui.add(Label::new("Name⬇").selectable(false));
											}
										}).1.clicked() {
											sort_ascending = !sort_ascending;
										}
									})
									.body(|body| {
										body.rows(15.0, filtered.len(), |mut row| {
											let (entry, kind) = &filtered[row.index()];

											if row.col(|ui| {
												if *kind != FileEntryKind::File {
													paint_bg(ui);
												}

												ui.add(Label::new(entry.to_str().unwrap()).selectable(false).truncate());
											}).1.clicked() {
												match kind {
													FileEntryKind::Parent => {
														if let Some(oldpath) = path {
															if let Some(parent) = oldpath.parent() {
																*path = Some(parent.into())
															} else {
																*path = None;
															}
														}
														path_dirty = true;
													}

													FileEntryKind::Dir => {
														if let Some(path) = path {
															path.push(entry);
														} else {
															// convert drive letter to path
															let mut drivepath = entry.clone();
															drivepath.push(std::path::MAIN_SEPARATOR_STR);
															*path = Some(drivepath.into());
														}

														path_dirty = true;
													}

													FileEntryKind::File => {
														let Some(path) = path else {
															unreachable!()
														};

														let mut filepath = path.clone();
														filepath.push(entry);
														new_sf = Some(filepath);
													}
												}
											};
										});
									});
								});
						});

						ui.with_layout(Layout::left_to_right(Align::TOP), |ui| {
							let home = ui.put(Rect::from_min_size(Pos2::new(8.0, 380.0), Vec2::new(8.0, 0.0)), Button::new(RichText::new("~").size(10.0)).small());

							if home.clicked() {
								*path = Some(get_library_path());
								path_dirty = true;
							}

							home.on_hover_text("return to library root");

							if let Some(path) = path {
								ui.add(
									Label::new(
										RichText::new(path.to_string_lossy()).size(10.0).color(Color32::GRAY)
									).selectable(false).truncate()
								);
							}
						});

						if path_dirty {
							state.refresh_browser_list();
						}

						if sort_ascending != state.file_sort_ascending {
							state.file_sort_ascending = sort_ascending;
							state.update_browser_filter();
						}

						if let Some(new_sf) = new_sf {
							state.loading = true;
							state.page = SfltUiPage::Main;
							state.page_dirty = true;
							state.load_error = None;
							state.push_message(SfltUiMessage::SoundFontChanged(new_sf, active_channel));
						}
					}
				}



				SfltUiPage::Settings => {
					ui.allocate_ui_with_layout(Vec2::new(0.0, 0.0), Layout::left_to_right(Align::TOP), |ui| {
						if ui.button("back").clicked() {
							state.page = SfltUiPage::Main;
							state.page_dirty = true;
						}

						ui.separator();

						let subpages = [
							(SettingsSubpage::Global, "global"),
							(SettingsSubpage::Engine, "tweaks"),
							#[cfg(not(feature = "fl"))]
							(SettingsSubpage::Instance, "wrapper"),
						];

						for subpage in subpages {
							if ui.selectable_label(state.settings_subpage == subpage.0, subpage.1).clicked() {
								state.settings_subpage = subpage.0;
							}
						}
					});

					ui.separator();

					match state.settings_subpage {
						#[cfg(not(feature = "fl"))]
						SettingsSubpage::Instance => {
							// instance settings (vst3/clap/au only)
							ui.columns(2, |columns| {
								columns[0].with_layout(Layout::top_down(Align::LEFT), |ui| {
									let mut bend_range = state.state.bend_range() as u8;
									let mut polyphony = state.state.polyphony();

									// bend range
									ui.add(Label::new("pitch bend range").selectable(false));

									let bend_val = sflt_drag_value(ui, state, &mut bend_range, None, 0, 48);

									if bend_val.changed() {
										bend_range = bend_range.min(48);
										state.push_message(SfltUiMessage::BendRangeChanged(bend_range as f32));
									}

									global_context_menu(
										&bend_val,
										|value| state.push_message(SfltUiMessage::BendRangeChanged(value as f32)),
										2
									);

									// polyphony
									ui.add(Label::new("polyphony").selectable(false));

									let poly = sflt_drag_value(ui, state, &mut polyphony, None, 1, 128);

									if poly.changed() {
										polyphony = polyphony.clamp(1, 128);
										state.push_message(SfltUiMessage::PolyphonyChanged(polyphony));
									}

									global_context_menu(
										&poly,
										|value| state.push_message(SfltUiMessage::PolyphonyChanged(value)),
										32
									);
									
									ui.add_space(8.0);

									{
										let mut value = state.state.master_tuning();
										let knob = Knob::new(&mut value, -12.0, 12.0, knob::KnobStyle::Wiper)
											.with_label("master tuning", knob::LabelPosition::Right)
											.with_delta_offset(-state.mouse_offset.y)
											.with_size(35.0);

										let mut response = ui.add(knob);

										widget_scroll_behaviour(ui, &mut value, &mut response, -12.0, 12.0, None);

										if response.changed() {
											value = value.clamp(-12.0, 12.0);
											state.push_message(SfltUiMessage::MasterTuningChanged(value));
										}
										
										let value_int = (value * 100.0) as i32;
										let text = if value_int > 0 {
											format!("Master tuning: +{} cents", value_int)
										} else {
											format!("Master tuning: {} cents", value_int)
										};

										if response.dragged() {
											sflt_widget_wrap(&response, ui, state);
											hint = HintText::Custom(text);
										}
										else if response.hovered() {
											hint = HintText::Custom(text);
										}

										if response.drag_stopped() {
											if let Some(own_handle) = state.own_handle && state.capture_mouse {
												jump_cursor(own_handle, response.rect.center(), ui.ctx().pixels_per_point());
												state.did_mouse_jump = true;
											}
										}

										global_context_menu(
											&response,
											|value| state.push_message(SfltUiMessage::MasterTuningChanged(value)),
											0.0
										);
									}
								});

								columns[1].with_layout(Layout::top_down(Align::LEFT), |ui| {
									let mut mono_mode = state.state.monophonic();
									let mut value = state.state.glide_time();

									ui.add_space(4.0);

									{
										let response = ui.add(Checkbox::new(&mut mono_mode, "legato"));

										if response.changed() {
											state.push_message(SfltUiMessage::MonophonicChanged(mono_mode));
										}
									}

									ui.add_space(8.0);

									{
										let knob = Knob::new(&mut value, -10000.0, 6000.0, knob::KnobStyle::Wiper)
											.with_label("slide length", knob::LabelPosition::Right)
											.with_delta_offset(-state.mouse_offset.y)
											.with_size(35.0);

										let mut response = ui.add_enabled(mono_mode, knob);

										widget_scroll_behaviour(ui, &mut value, &mut response, -10000.0, 6000.0, None);

										if response.changed() {
											value = value.clamp(-10000.0, 6000.0);
											state.push_message(SfltUiMessage::GlideTimeChanged(value));
										}

										if response.dragged() {
											sflt_widget_wrap(&response, ui, state);
											hint = HintText::Custom(format!("Slide length: {:.2}s", timecents_to_secs(value as f64)));
										}
										else if response.hovered() {
											hint = HintText::Custom(format!("Slide length: {:.2}s", timecents_to_secs(value as f64)));
										}

										if response.drag_stopped() {
											if let Some(own_handle) = state.own_handle && state.capture_mouse {
												jump_cursor(own_handle, response.rect.center(), ui.ctx().pixels_per_point());
												state.did_mouse_jump = true;
											}
										}

										global_context_menu(
											&response,
											|value| state.push_message(SfltUiMessage::GlideTimeChanged(value)),
											-10000.0
										);
									}
								});
							});

						}


						SettingsSubpage::Engine => {
							ui.add(Label::new("engine tweaks").selectable(false));

							for i in 0..EngineTweak::EngineTweakMax as u8 {
								let Ok(tweak) = EngineTweak::try_from(i) else {
									continue
								};

								let desc = match tweak {
									EngineTweak::MidiMappingFix => "midi cc mapping fix",
									EngineTweak::OverrideHoldWithDecay => "override envelope hold with decay",
									EngineTweak::SkipShortEnvPhases => "skip short envelope phases",
									EngineTweak::FilterFix => "filter processing fixes",
									EngineTweak::FilterQCap => "cap filter resonance at 24dB",

									EngineTweak::EngineTweakMax => "",
								};

								let mut tweak_enabled = state.state.tweak(tweak);

								if ui.checkbox(&mut tweak_enabled, desc).changed() {
									state.push_message(SfltUiMessage::EngineTweakChanged(tweak, tweak_enabled));
								}
							}
						}


						#[allow(unused_mut)]
						SettingsSubpage::Global => {
							let mut config_stale = false;

							let lock = CONFIG.read().unwrap();

							let mut rename_mode = lock.rename_mode;
							let mut scale_factor = lock.scale_factor;
							let mut library_path = lock.library_path.to_string_lossy().to_string();
							let mut capture_mouse = lock.capture_mouse;
							let mut auto_find_preset = lock.auto_find_preset;
							let mut use_24_bit_samples = lock.use_24_bit_samples;

							drop(lock);

							let liblabel = ui.add(Label::new("library path").selectable(false));

							if liblabel.hovered() {
								hint = HintText::Hover("Where to browse & locate SoundFonts");
							}

							ui.with_layout(Layout::left_to_right(Align::TOP), |ui| {
								ui.style_mut().visuals.extreme_bg_color = Color32::BLACK;

								if ui.add(TextEdit::singleline(&mut library_path)).changed() {
									config_stale = true;
								}

								let liblocator = ui.button("..");

								if liblocator.clicked() {
									state.push_message(SfltUiMessage::LibraryPathLocator);
								}

								if liblocator.hovered() {
									hint = HintText::Hover("Use native folder browser");
								}
							});

							#[cfg(feature = "fl")]
							{
								ui.add_space(4.0);

								ui.add(Label::new("rename mode").selectable(false));

								let modes = [
									(RenameMode::None,      "none",   "Don't rename channel"),
									(RenameMode::SoundFont, "file",   "Rename channel to match SoundFont"),
									(RenameMode::Preset,    "preset", "Rename channel to match preset"),
								];

								let response = ComboBox::from_id_salt("rename mode")
									.selected_text(rename_mode.to_string())
									.show_ui(ui, |ui| {
										for mode in modes {
											let response = ui.selectable_value(&mut rename_mode, mode.0, mode.1);

											if response.clicked() {
												config_stale = true;
											}

											if response.hovered() {
												hint = HintText::Hover(mode.2);
											}
										}
									});

								if response.response.hovered() {
									hint = HintText::Hover("Automatically change name in channel rack");
								}
							}

							// workaround for system scale factor not working correctly in fpsdk
							#[cfg(all(feature = "fl", target_os = "windows"))]
							{
								ui.add_space(4.0);

								ui.add(Label::new("window scaling (reopen to apply)").selectable(false));

								ComboBox::from_id_salt("window scaling")
									.selected_text(format!("{scale_factor}%"))
									.show_ui(ui, |ui| {
										if ui.selectable_value(&mut scale_factor, 100, "100%").clicked() {
											config_stale = true;
										}
										if ui.selectable_value(&mut scale_factor, 125, "125%").clicked() {
											config_stale = true;
										}
										if ui.selectable_value(&mut scale_factor, 150, "150%").clicked() {
											config_stale = true;
										}
										if ui.selectable_value(&mut scale_factor, 175, "175%").clicked() {
											config_stale = true;
										}
										if ui.selectable_value(&mut scale_factor, 200, "200%").clicked() {
											config_stale = true;
										}
									});
								}

							{
								ui.add_space(4.0);

								let response = ui.add(Checkbox::new(&mut capture_mouse, "capture mouse when dragging"));

								if response.changed() {
									config_stale = true;
								}

								if !response.dragged() && response.hovered() {
									hint = HintText::Hover("May cause issues in some hosts");
								}
							}

							{
								ui.add_space(4.0);

								let response = ui.add(Checkbox::new(&mut auto_find_preset, "find valid preset on load"));

								if response.changed() {
									config_stale = true;
								}

								if !response.dragged() && response.hovered() {
									hint = HintText::Hover("Use bank's first preset, if none at current patch");
								}
							}

							{
								ui.add_space(4.0);

								let response = ui.add(Checkbox::new(&mut use_24_bit_samples, "use 24-bit samples if available"));

								if response.changed() {
									config_stale = true;
								}

								if !response.dragged() && response.hovered() {
									hint = HintText::Hover("Load optional 24-bit sample data (applies on SoundFont load)");
								}
							}

							if config_stale {
								let mut lock = CONFIG.write().unwrap();

								lock.library_path = library_path.into();
								lock.rename_mode = rename_mode;
								lock.capture_mouse = capture_mouse;
								lock.scale_factor = scale_factor;
								lock.auto_find_preset = auto_find_preset;
								lock.use_24_bit_samples = use_24_bit_samples;

								drop(lock);

								let _ = crate::config::save_config();
							}
						}
					}

					#[cfg(not(feature = "fl"))]
					{
						write_hint_text(ui, state, &hint, Pos2::new(361.0, 377.0));
					}
				}



				SfltUiPage::About => {
					if ui.button("back").clicked() {
						state.page = SfltUiPage::Main;
						state.page_dirty = true;
					}

					if page_dirty {
						state.motd = MOTD_LIST[random::<u32>() as usize % MOTD_LIST.len()];
					}

					ui.with_layout(Layout::top_down(Align::Center), |ui| {
						ui.add(state.logo_about.clone());
						ui.separator();
						ui.add(
							Label::new(
								RichText::new(format!("BETA v{VERSION} / by ash taylor"))
									.size(16.0)
									.color(Color32::GRAY)
							)
							.selectable(false)
						);

						ui.hyperlink_to(
							RichText::new(SFLT_LINK)
								.italics()
								.color(Color32::DARK_GRAY),
							SFLT_LINK
						);

						ui.add_space(10.0);

						ui.collapsing("legal info", |ui| {

							ui.add(
								Label::new(
									RichText::new(
										"SFLT is released under the GNU General Public License v3.0. You should have received a copy of the source code along with the plugin. If not, you can acquire it via the link above.\n\nVST is a trademark of Steinberg Media Technologies GmbH, registered in Europe and other countries."
									)
										.size(10.0)
										.color(Color32::DARK_GRAY)
								)
								.selectable(false)
							);
						});

						// write MOTD
						{
							let painter = ui.painter();
							let mut font = FontId::default();

							font.size = 12.0;

							let galley = painter.layout_no_wrap(state.motd.to_string(), font, Color32::DARK_GRAY);

							let mut pos = Pos2::new(WINDOW_WIDTH as f32 / 2.0, WINDOW_HEIGHT as f32);

							pos.x -= galley.size().x / 2.0;
							pos.y -= 16.0;

							painter.galley(pos, galley, Color32::DARK_GRAY);
						}
					});
				}
			}
		});

		if preview_note != state.preview_note {
			state.preview_note = preview_note;
			state.push_message(SfltUiMessage::SetPreviewNote(preview_note));
		}

		if hint != state.last_hint {
			state.push_message(SfltUiMessage::HintChanged(hint.clone()));
			state.last_hint = hint;
		}
	});

	ctx.request_repaint();
}


pub struct FuckYou(pub RawWindowHandle);

unsafe impl HasRawWindowHandle for FuckYou {
	fn raw_window_handle(&self) -> RawWindowHandle {
		self.0
	}
}
