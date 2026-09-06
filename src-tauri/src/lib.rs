mod lut;

use image::{DynamicImage, ImageFormat, ImageReader};
use lut::{scan_directory, CubeLut, LutDescriptor};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Instant, SystemTime},
};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    lut_directory: Option<String>,
}

#[derive(Clone)]
struct CachedLut {
    modified: Option<SystemTime>,
    file_size: u64,
    lut: Arc<CubeLut>,
}

#[derive(Clone, Default)]
pub struct AppState {
    settings: Arc<RwLock<AppSettings>>,
    lut_cache: Arc<RwLock<HashMap<PathBuf, CachedLut>>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LutCatalog {
    directory: String,
    luts: Vec<LutDescriptor>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInfo {
    path: String,
    width: u32,
    height: u32,
    color_type: String,
    format: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessImageRequest {
    input_path: String,
    lut_path: String,
    output_path: String,
    intensity: f32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessImageResult {
    output_path: String,
    width: u32,
    height: u32,
    elapsed_ms: u128,
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    state
        .settings
        .read()
        .map(|value| value.clone())
        .map_err(|_| "Settings lock was poisoned".into())
}

#[tauri::command]
fn set_lut_directory(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<LutCatalog, String> {
    let directory = canonical_directory(Path::new(&path))?;
    let luts = scan_directory(&directory)?;
    let settings = AppSettings {
        lut_directory: Some(directory.to_string_lossy().into_owned()),
    };
    save_settings(&app, &settings)?;
    *state
        .settings
        .write()
        .map_err(|_| "Settings lock was poisoned".to_string())? = settings;
    state
        .lut_cache
        .write()
        .map_err(|_| "LUT cache lock was poisoned".to_string())?
        .clear();
    Ok(LutCatalog {
        directory: directory.to_string_lossy().into_owned(),
        luts,
    })
}

#[tauri::command]
fn list_luts(state: State<'_, AppState>) -> Result<LutCatalog, String> {
    let directory = configured_lut_directory(&state)?;
    let luts = scan_directory(&directory)?;
    Ok(LutCatalog {
        directory: directory.to_string_lossy().into_owned(),
        luts,
    })
}

#[tauri::command]
fn choose_image(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .add_filter(
            "Images",
            &[
                "png", "jpg", "jpeg", "webp", "avif", "tif", "tiff", "bmp", "gif", "qoi", "tga",
            ],
        )
        .blocking_pick_file()
        .and_then(|path| path.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
fn choose_lut_directory(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .blocking_pick_folder()
        .and_then(|path| path.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
fn choose_output_path(app: AppHandle, suggested_name: Option<String>) -> Option<String> {
    let mut dialog = app.dialog().file().add_filter("PNG image", &["png"]);
    if let Some(name) = suggested_name.filter(|name| !name.trim().is_empty()) {
        dialog = dialog.set_file_name(name);
    }
    dialog
        .blocking_save_file()
        .and_then(|path| path.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
fn inspect_image(input_path: String) -> Result<ImageInfo, String> {
    let path = Path::new(&input_path);
    let reader = ImageReader::open(path)
        .map_err(|error| format!("Could not open image '{}': {error}", path.display()))?
        .with_guessed_format()
        .map_err(|error| format!("Could not identify image '{}': {error}", path.display()))?;
    let format = reader
        .format()
        .map(|value| format!("{value:?}"))
        .unwrap_or_else(|| "Unknown".into());
    let image = reader
        .decode()
        .map_err(|error| format!("Could not decode image '{}': {error}", path.display()))?;
    Ok(ImageInfo {
        path: path.to_string_lossy().into_owned(),
        width: image.width(),
        height: image.height(),
        color_type: format!("{:?}", image.color()),
        format,
    })
}

#[tauri::command]
async fn process_image(
    state: State<'_, AppState>,
    request: ProcessImageRequest,
) -> Result<ProcessImageResult, String> {
    if !request.intensity.is_finite() || !(0.0..=1.0).contains(&request.intensity) {
        return Err("Intensity must be a number between 0 and 1".into());
    }
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || process_image_blocking(&state, request))
        .await
        .map_err(|error| format!("Image processing task failed: {error}"))?
}

fn process_image_blocking(
    state: &AppState,
    request: ProcessImageRequest,
) -> Result<ProcessImageResult, String> {
    let started = Instant::now();
    let lut_path = canonical_lut_path(state, Path::new(&request.lut_path))?;
    let lut = cached_lut(state, &lut_path)?;
    let input_path = Path::new(&request.input_path);
    let output_path = Path::new(&request.output_path);
    let canonical_input = input_path.canonicalize().map_err(|error| {
        format!(
            "Could not access input image '{}': {error}",
            input_path.display()
        )
    })?;
    let overwrites_input = output_path
        .canonicalize()
        .is_ok_and(|canonical_output| canonical_output == canonical_input);
    if overwrites_input {
        return Err("Input and output paths must be different".into());
    }
    if !output_path.parent().is_some_and(Path::is_dir) {
        return Err("The output directory does not exist".into());
    }
    let output_format = ImageFormat::from_path(output_path).map_err(|_| {
        "Unsupported output extension. Use PNG, JPEG, WebP, AVIF, TIFF, BMP, QOI, or TGA."
            .to_string()
    })?;
    let image = ImageReader::open(input_path)
        .map_err(|error| format!("Could not open image '{}': {error}", input_path.display()))?
        .with_guessed_format()
        .map_err(|error| {
            format!(
                "Could not identify image '{}': {error}",
                input_path.display()
            )
        })?
        .decode()
        .map_err(|error| format!("Could not decode image '{}': {error}", input_path.display()))?;
    let (width, height) = (image.width(), image.height());
    let mut pixels = image.into_rgba8();
    let intensity = request.intensity;
    pixels.as_mut().par_chunks_exact_mut(4).for_each(|pixel| {
        let original = [
            pixel[0] as f32 / 255.0,
            pixel[1] as f32 / 255.0,
            pixel[2] as f32 / 255.0,
        ];
        let transformed = lut.sample(original);
        for channel in 0..3 {
            let value = original[channel] + (transformed[channel] - original[channel]) * intensity;
            pixel[channel] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    });
    let file = fs::File::create(output_path).map_err(|error| {
        format!(
            "Could not create output '{}': {error}",
            output_path.display()
        )
    })?;
    let mut writer = BufWriter::new(file);
    DynamicImage::ImageRgba8(pixels)
        .write_to(&mut writer, output_format)
        .map_err(|error| {
            format!(
                "Could not encode output '{}': {error}",
                output_path.display()
            )
        })?;
    writer.flush().map_err(|error| {
        format!(
            "Could not finish output '{}': {error}",
            output_path.display()
        )
    })?;
    Ok(ProcessImageResult {
        output_path: output_path.to_string_lossy().into_owned(),
        width,
        height,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    let canonical = path.canonicalize().map_err(|error| {
        format!(
            "Could not access LUT directory '{}': {error}",
            path.display()
        )
    })?;
    if !canonical.is_dir() {
        return Err(format!("'{}' is not a directory", canonical.display()));
    }
    Ok(canonical)
}

fn configured_lut_directory(state: &AppState) -> Result<PathBuf, String> {
    let path = state
        .settings
        .read()
        .map_err(|_| "Settings lock was poisoned".to_string())?
        .lut_directory
        .clone()
        .ok_or_else(|| "Choose a LUT directory first".to_string())?;
    canonical_directory(Path::new(&path))
}

fn canonical_lut_path(state: &AppState, path: &Path) -> Result<PathBuf, String> {
    let root = configured_lut_directory(state)?;
    let path = path
        .canonicalize()
        .map_err(|error| format!("Could not access LUT '{}': {error}", path.display()))?;
    if !path.starts_with(&root) {
        return Err("The selected LUT is outside the configured LUT directory".into());
    }
    if !path
        .extension()
        .is_some_and(|value| value.to_string_lossy().eq_ignore_ascii_case("cube"))
    {
        return Err("Only .cube LUT files are supported".into());
    }
    Ok(path)
}

fn cached_lut(state: &AppState, path: &Path) -> Result<Arc<CubeLut>, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Could not inspect LUT '{}': {error}", path.display()))?;
    let modified = metadata.modified().ok();
    if let Some(cached) = state
        .lut_cache
        .read()
        .map_err(|_| "LUT cache lock was poisoned".to_string())?
        .get(path)
        .filter(|cached| cached.modified == modified && cached.file_size == metadata.len())
    {
        return Ok(Arc::clone(&cached.lut));
    }
    let lut = Arc::new(CubeLut::parse(path)?);
    state
        .lut_cache
        .write()
        .map_err(|_| "LUT cache lock was poisoned".to_string())?
        .insert(
            path.to_owned(),
            CachedLut {
                modified,
                file_size: metadata.len(),
                lut: Arc::clone(&lut),
            },
        );
    Ok(lut)
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join("settings.json"))
        .map_err(|error| format!("Could not locate application settings directory: {error}"))
}

fn load_settings(app: &AppHandle) -> AppSettings {
    let Ok(path) = settings_path(app) else {
        return AppSettings::default();
    };
    let Ok(bytes) = fs::read(path) else {
        return AppSettings::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

fn save_settings(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    let path = settings_path(app)?;
    let directory = path
        .parent()
        .ok_or_else(|| "Invalid settings path".to_string())?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("Could not create application settings directory: {error}"))?;
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("Could not serialize settings: {error}"))?;
    fs::write(&path, bytes).map_err(|error| format!("Could not save settings: {error}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let settings = load_settings(app.handle());
            app.manage(AppState {
                settings: Arc::new(RwLock::new(settings)),
                lut_cache: Arc::default(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_lut_directory,
            list_luts,
            choose_image,
            choose_lut_directory,
            choose_output_path,
            inspect_image,
            process_image
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    use std::time::UNIX_EPOCH;

    #[test]
    fn processes_an_image_with_a_cube_lut() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("luty-test-{}-{nonce}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let input = directory.join("input.png");
        let output = directory.join("output.png");
        let cube = directory.join("invert.cube");
        RgbaImage::from_pixel(1, 1, Rgba([255, 0, 0, 137]))
            .save(&input)
            .unwrap();
        fs::write(
            &cube,
            "LUT_3D_SIZE 2\n1 1 1\n0 1 1\n1 0 1\n0 0 1\n1 1 0\n0 1 0\n1 0 0\n0 0 0\n",
        )
        .unwrap();
        let state = AppState {
            settings: Arc::new(RwLock::new(AppSettings {
                lut_directory: Some(directory.to_string_lossy().into_owned()),
            })),
            lut_cache: Arc::default(),
        };
        process_image_blocking(
            &state,
            ProcessImageRequest {
                input_path: input.to_string_lossy().into_owned(),
                lut_path: cube.to_string_lossy().into_owned(),
                output_path: output.to_string_lossy().into_owned(),
                intensity: 1.0,
            },
        )
        .unwrap();
        let pixel = image::open(&output).unwrap().into_rgba8().get_pixel(0, 0).0;
        assert_eq!(pixel, [0, 255, 255, 137]);
        fs::remove_dir_all(directory).unwrap();
    }
}
