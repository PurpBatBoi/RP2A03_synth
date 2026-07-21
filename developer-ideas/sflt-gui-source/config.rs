use std::{error::Error, fmt::Display, fs::{self, read_to_string, File}, io::{self, Write}, path::PathBuf, sync::{Arc, LazyLock, RwLock}};

#[cfg(target_os = "windows")]
use known_folders::get_known_folder_path;

#[derive(Copy, Clone, Default, PartialEq)]
pub enum RenameMode {
	#[default]
	None = 0,
	SoundFont = 1,
	Preset = 2,
}

impl Display for RenameMode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			RenameMode::None => write!(f, "none"),
			RenameMode::SoundFont => write!(f, "file"),
			RenameMode::Preset => write!(f, "preset"),
		}
	}
}

const CONFIG_VERSION: u32 = 7;

pub static CONFIG_PATH: LazyLock<PathBuf> = LazyLock::new(|| get_config_path());
pub static CONFIG: LazyLock<Arc<RwLock<SfltSharedConfig>>> = LazyLock::new(|| Arc::new(RwLock::new(SfltSharedConfig::new())));

#[derive(Default)]
pub struct SfltSharedConfig {
	config_version: u32,

	pub library_path: PathBuf,

	pub skin: u32,

	pub rename_mode: RenameMode,

	pub threaded_loading: bool, // TODO: reimplement

	pub capture_mouse: bool,

	pub scale_factor: i32,

	pub auto_find_preset: bool,

	pub use_24_bit_samples: bool,
}

impl SfltSharedConfig {
	fn new() -> Self {
		SfltSharedConfig {
			config_version: CONFIG_VERSION,
			threaded_loading: true,
			capture_mouse: true,
			scale_factor: 100,
			auto_find_preset: true,
			use_24_bit_samples: true,
			..Default::default()
		}
	}
}

fn get_default_library_path() -> PathBuf {
	#[cfg(target_os = "macos")]
	{
		PathBuf::from("/Users/Shared/Soundfonts")
	}

	#[cfg(not(target_os = "macos"))]
	{
		let path = &*CONFIG_PATH;
		path.join("Soundfonts")
	}
}

pub fn get_config_path() -> PathBuf {
	#[cfg(target_os = "windows")]
	{
		let mut documents = get_known_folder_path(known_folders::KnownFolder::Documents).unwrap();

		documents.push("SFLT");

		documents
	}

	#[cfg(target_os = "macos")]
	{
		let mut home = std::env::home_dir().unwrap();
		home.push("Library");
		home.push("Application Support");
		home.push("SFLT");
		home
	}

	#[cfg(target_os = "linux")]
	{
		let mut home = std::env::home_dir().unwrap();

		home.push(".sflt");

		home
	}
}


pub fn load_config() -> Result<(), Box<dyn Error>> {
	let path = &*CONFIG_PATH;

	if !path.exists() {
		// no config to load here, init the directory structure and get out
		return init_config()
	}

	let Ok(file) = read_to_string(CONFIG_PATH.join(".config")) else {
		return init_config()
	};

	let mut lines = file.lines();

	let mut config = CONFIG.write()?;
	let version: u32 = next_line(&mut lines)?.parse()?;

	if version > config.config_version {
		return Err(io::Error::new(io::ErrorKind::InvalidData, "unsupported .config version!").into());
	}

	config.library_path = next_line(&mut lines)?.into();
	config.skin = next_line(&mut lines)?.parse()?;

	if version >= 2 {
		config.rename_mode = match next_line(&mut lines)?.parse()? {
			0 => RenameMode::None,
			1 => RenameMode::SoundFont,
			2 => RenameMode::Preset,
			_ => return Err(io::Error::new(io::ErrorKind::InvalidData, "unsupported .config version!").into())
		}
	}

	if version >= 3 {
		config.threaded_loading = next_line(&mut lines)?.parse::<i32>()? != 0;
	}

	if version >= 4 {
		config.capture_mouse = next_line(&mut lines)?.parse::<i32>()? != 0;
	}

	if version >= 5 {
		config.scale_factor = next_line(&mut lines)?.parse()?;
	}

	if version >= 6 {
		config.auto_find_preset = next_line(&mut lines)?.parse::<i32>()? != 0;
	}

	if version >= 7 {
		config.use_24_bit_samples = next_line(&mut lines)?.parse::<i32>()? != 0;
	}

	Ok(())
}


pub fn init_config() -> Result<(), Box<dyn Error>> {
	let sfpath = get_default_library_path();

	let _ = fs::create_dir_all(&sfpath);

	{
		let mut config = CONFIG.write()?;
		config.library_path = sfpath;
	}

	save_config()?;
	Ok(())
}


fn next_line<'a>(lines: &mut std::str::Lines<'a>) -> io::Result<&'a str> {
	match lines.next() {
		Some(line) => Ok(line),
		None => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "unexpected EOF!"))
	}
}


pub fn save_config() -> std::io::Result<()> {
	let mut file = File::create(CONFIG_PATH.join(".config"))?;
	let config = CONFIG.read().unwrap();

	writeln!(file, "{}", config.config_version)?;
	writeln!(file, "{}", config.library_path.display())?;
	writeln!(file, "{}", config.skin)?;
	writeln!(file, "{}", config.rename_mode as i32)?;
	writeln!(file, "{}", config.threaded_loading as i32)?;
	writeln!(file, "{}", config.capture_mouse as i32)?;
	writeln!(file, "{}", config.scale_factor)?;
	writeln!(file, "{}", config.auto_find_preset as i32)?;
	writeln!(file, "{}", config.use_24_bit_samples as i32)?;

	Ok(())
}