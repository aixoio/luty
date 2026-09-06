mod lut;

use image::{metadata::Orientation, DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use lut::{scan_directory, CubeLut, LutDescriptor};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{ipc::Channel, AppHandle, Manager, State};
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessProgress {
    stage: &'static str,
    percent: u8,
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
async fn choose_image(app: AppHandle) -> Result<Option<String>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(
            "Images",
            &[
                "png", "jpg", "jpeg", "webp", "avif", "tif", "tiff", "bmp", "gif", "hdr", "qoi",
                "tga",
            ],
        )
        .pick_file(move |path| {
            let _ = sender.send(dialog_path_to_string(path));
        });
    receiver
        .await
        .map_err(|_| "The image picker closed unexpectedly".to_string())
}

#[tauri::command]
async fn choose_images(app: AppHandle) -> Result<Vec<String>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(
            "Images",
            &[
                "png", "jpg", "jpeg", "webp", "avif", "tif", "tiff", "bmp", "gif", "hdr", "qoi",
                "tga",
            ],
        )
        .pick_files(move |paths| {
            let paths = paths
                .unwrap_or_default()
                .into_iter()
                .filter_map(|path| path.into_path().ok())
                .map(|path| path.to_string_lossy().into_owned())
                .collect();
            let _ = sender.send(paths);
        });
    receiver
        .await
        .map_err(|_| "The image picker closed unexpectedly".to_string())
}

#[tauri::command]
async fn choose_lut_directory(app: AppHandle) -> Result<Option<String>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |path| {
        let _ = sender.send(dialog_path_to_string(path));
    });
    receiver
        .await
        .map_err(|_| "The folder picker closed unexpectedly".to_string())
}

#[tauri::command]
async fn choose_output_path(
    app: AppHandle,
    suggested_name: Option<String>,
) -> Result<Option<String>, String> {
    let mut dialog = app.dialog().file().add_filter("PNG image", &["png"]);
    if let Some(name) = suggested_name.filter(|name| !name.trim().is_empty()) {
        dialog = dialog.set_file_name(name);
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    dialog.save_file(move |path| {
        let _ = sender.send(dialog_path_to_string(path));
    });
    receiver
        .await
        .map_err(|_| "The export picker closed unexpectedly".to_string())
}

#[tauri::command]
async fn choose_output_directory(app: AppHandle) -> Result<Option<String>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |path| {
        let _ = sender.send(dialog_path_to_string(path));
    });
    receiver
        .await
        .map_err(|_| "The export folder picker closed unexpectedly".to_string())
}

#[tauri::command]
fn available_output_paths(directory: String, names: Vec<String>) -> Result<Vec<String>, String> {
    let directory = Path::new(&directory)
        .canonicalize()
        .map_err(|error| format!("Could not access output directory '{directory}': {error}"))?;
    if !directory.is_dir() {
        return Err("The selected output location is not a directory".into());
    }

    let mut reserved = HashSet::new();
    let mut paths = Vec::with_capacity(names.len());
    for name in names {
        let requested = Path::new(&name);
        if requested.file_name() != Some(requested.as_os_str()) {
            return Err("An output filename contained an invalid path".into());
        }
        let stem = requested
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("image");
        let extension = requested
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("png");
        let mut candidate = directory.join(&name);
        let mut suffix = 2_u32;
        while candidate.exists() || reserved.contains(&candidate) {
            candidate = directory.join(format!("{stem}-{suffix}.{extension}"));
            suffix += 1;
        }
        reserved.insert(candidate.clone());
        paths.push(candidate.to_string_lossy().into_owned());
    }
    Ok(paths)
}

fn dialog_path_to_string(path: Option<tauri_plugin_dialog::FilePath>) -> Option<String> {
    path.and_then(|path| path.into_path().ok())
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
    let mut decoder = reader
        .into_decoder()
        .map_err(|error| format!("Could not inspect image '{}': {error}", path.display()))?;
    let (mut width, mut height) = decoder.dimensions();
    let color_type = format!("{:?}", decoder.color_type());
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    if matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    ) {
        std::mem::swap(&mut width, &mut height);
    }
    Ok(ImageInfo {
        path: path.to_string_lossy().into_owned(),
        width,
        height,
        color_type,
        format,
    })
}

#[tauri::command]
async fn process_image(
    state: State<'_, AppState>,
    request: ProcessImageRequest,
    progress: Channel<ProcessProgress>,
) -> Result<ProcessImageResult, String> {
    if !request.intensity.is_finite() || !(0.0..=1.0).contains(&request.intensity) {
        return Err("Intensity must be a number between 0 and 1".into());
    }
    let state = state.inner().clone();
    send_progress(&progress, "Preparing image", 5);
    tauri::async_runtime::spawn_blocking(move || {
        process_image_blocking_with_progress(&state, request, Some(&progress))
    })
    .await
    .map_err(|error| format!("Image processing task failed: {error}"))?
}

#[tauri::command]
async fn render_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    input_path: String,
    lut_path: String,
    intensity: f32,
) -> Result<ProcessImageResult, String> {
    if !intensity.is_finite() || !(0.0..=1.0).contains(&intensity) {
        return Err("Intensity must be a number between 0 and 1".into());
    }
    let cache_directory = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("Could not locate preview cache: {error}"))?
        .join("previews");
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        render_preview_blocking(&state, &input_path, &lut_path, intensity, &cache_directory)
    })
    .await
    .map_err(|error| format!("Preview task failed: {error}"))?
}

#[tauri::command]
async fn render_source_preview(
    app: AppHandle,
    input_path: String,
) -> Result<ProcessImageResult, String> {
    let cache_directory = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("Could not locate preview cache: {error}"))?
        .join("previews");
    tauri::async_runtime::spawn_blocking(move || {
        render_source_preview_blocking(Path::new(&input_path), &cache_directory)
    })
    .await
    .map_err(|error| format!("Source preview task failed: {error}"))?
}

#[cfg(test)]
fn process_image_blocking(
    state: &AppState,
    request: ProcessImageRequest,
) -> Result<ProcessImageResult, String> {
    process_image_blocking_with_progress(state, request, None)
}

fn process_image_blocking_with_progress(
    state: &AppState,
    request: ProcessImageRequest,
    progress: Option<&Channel<ProcessProgress>>,
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
    send_optional_progress(progress, "Decoding to SDR", 18);
    let image = decode_image(input_path)?;
    let (width, height) = (image.width(), image.height());
    send_optional_progress(progress, "Applying LUT", 38);
    let pixels = apply_lut(image, &lut, request.intensity);
    send_optional_progress(progress, "Encoding image", 82);
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
    send_optional_progress(progress, "Export complete", 100);
    Ok(ProcessImageResult {
        output_path: output_path.to_string_lossy().into_owned(),
        width,
        height,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn render_preview_blocking(
    state: &AppState,
    input_path: &str,
    lut_path: &str,
    intensity: f32,
    cache_directory: &Path,
) -> Result<ProcessImageResult, String> {
    let started = Instant::now();
    let input_path = Path::new(input_path);
    let lut_path = canonical_lut_path(state, Path::new(lut_path))?;
    let lut = cached_lut(state, &lut_path)?;
    let metadata = fs::metadata(input_path).map_err(|error| {
        format!(
            "Could not inspect image '{}': {error}",
            input_path.display()
        )
    })?;
    let lut_metadata = fs::metadata(&lut_path)
        .map_err(|error| format!("Could not inspect LUT '{}': {error}", lut_path.display()))?;
    let mut hasher = DefaultHasher::new();
    // Invalidate previews made before decode-time orientation normalization.
    3_u8.hash(&mut hasher);
    input_path.hash(&mut hasher);
    lut_path.hash(&mut hasher);
    intensity.to_bits().hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .hash(&mut hasher);
    lut_metadata.len().hash(&mut hasher);
    lut_metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .hash(&mut hasher);
    fs::create_dir_all(cache_directory)
        .map_err(|error| format!("Could not create preview cache: {error}"))?;
    let output_path = cache_directory.join(format!("{:016x}.png", hasher.finish()));

    if output_path.is_file() {
        let (width, height) = image::image_dimensions(&output_path)
            .map_err(|error| format!("Could not inspect cached preview: {error}"))?;
        return Ok(ProcessImageResult {
            output_path: output_path.to_string_lossy().into_owned(),
            width,
            height,
            elapsed_ms: started.elapsed().as_millis(),
        });
    }

    let source_preview = render_source_preview_blocking(input_path, cache_directory)?;
    let image = decode_image(Path::new(&source_preview.output_path))?;
    let (width, height) = (image.width(), image.height());
    let pixels = apply_lut(image, &lut, intensity);
    DynamicImage::ImageRgba8(pixels)
        .save_with_format(&output_path, ImageFormat::Png)
        .map_err(|error| format!("Could not write preview: {error}"))?;
    Ok(ProcessImageResult {
        output_path: output_path.to_string_lossy().into_owned(),
        width,
        height,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn render_source_preview_blocking(
    input_path: &Path,
    cache_directory: &Path,
) -> Result<ProcessImageResult, String> {
    let started = Instant::now();
    let metadata = fs::metadata(input_path).map_err(|error| {
        format!(
            "Could not inspect image '{}': {error}",
            input_path.display()
        )
    })?;
    let mut hasher = DefaultHasher::new();
    "sdr-source-preview-v1".hash(&mut hasher);
    input_path.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .hash(&mut hasher);
    fs::create_dir_all(cache_directory)
        .map_err(|error| format!("Could not create preview cache: {error}"))?;
    let output_path = cache_directory.join(format!("source-{:016x}.png", hasher.finish()));

    if output_path.is_file() {
        let (width, height) = image::image_dimensions(&output_path)
            .map_err(|error| format!("Could not inspect cached source preview: {error}"))?;
        return Ok(ProcessImageResult {
            output_path: output_path.to_string_lossy().into_owned(),
            width,
            height,
            elapsed_ms: started.elapsed().as_millis(),
        });
    }

    let image = decode_image(input_path)?;
    let image = if image.width() > 1600 || image.height() > 1600 {
        image.thumbnail(1600, 1600)
    } else {
        image
    };
    let (width, height) = (image.width(), image.height());
    image
        .save_with_format(&output_path, ImageFormat::Png)
        .map_err(|error| format!("Could not write source preview: {error}"))?;
    Ok(ProcessImageResult {
        output_path: output_path.to_string_lossy().into_owned(),
        width,
        height,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn decode_image(path: &Path) -> Result<DynamicImage, String> {
    let reader = ImageReader::open(path)
        .map_err(|error| format!("Could not open image '{}': {error}", path.display()))?
        .with_guessed_format()
        .map_err(|error| format!("Could not identify image '{}': {error}", path.display()))?;
    let format = reader.format();
    let mut decoder = reader
        .into_decoder()
        .map_err(|error| format!("Could not decode image '{}': {error}", path.display()))?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder)
        .map_err(|error| format!("Could not decode image '{}': {error}", path.display()))?;
    image.apply_orientation(orientation);
    Ok(DynamicImage::ImageRgba8(convert_to_sdr(image, format)))
}

fn convert_to_sdr(image: DynamicImage, format: Option<ImageFormat>) -> image::RgbaImage {
    let is_scene_linear_hdr = matches!(
        &image,
        DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_)
    ) || matches!(format, Some(ImageFormat::Hdr | ImageFormat::OpenExr));
    if !is_scene_linear_hdr {
        return image.into_rgba8();
    }

    let (width, height) = (image.width(), image.height());
    let source = image.into_rgba32f().into_raw();
    let mut output = vec![0_u8; source.len()];
    output
        .par_chunks_exact_mut(4)
        .zip(source.par_chunks_exact(4))
        .for_each(|(target, pixel)| {
            let red = finite_positive(pixel[0]);
            let green = finite_positive(pixel[1]);
            let blue = finite_positive(pixel[2]);
            let luminance = 0.2126 * red + 0.7152 * green + 0.0722 * blue;
            let mapped_luminance = aces_filmic(luminance);
            let scale = if luminance > f32::EPSILON {
                mapped_luminance / luminance
            } else {
                0.0
            };
            target[0] = linear_to_srgb_u8(red * scale);
            target[1] = linear_to_srgb_u8(green * scale);
            target[2] = linear_to_srgb_u8(blue * scale);
            target[3] = (finite_positive(pixel[3]).clamp(0.0, 1.0) * 255.0).round() as u8;
        });
    image::RgbaImage::from_raw(width, height, output)
        .expect("RGBA conversion preserves image dimensions")
}

fn finite_positive(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn aces_filmic(value: f32) -> f32 {
    ((value * (2.51 * value + 0.03)) / (value * (2.43 * value + 0.59) + 0.14)).clamp(0.0, 1.0)
}

fn linear_to_srgb_u8(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let encoded = if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

fn apply_lut(image: DynamicImage, lut: &CubeLut, intensity: f32) -> image::RgbaImage {
    let mut pixels = image.into_rgba8();
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
    pixels
}

fn send_progress(channel: &Channel<ProcessProgress>, stage: &'static str, percent: u8) {
    let _ = channel.send(ProcessProgress { stage, percent });
}

fn send_optional_progress(
    channel: Option<&Channel<ProcessProgress>>,
    stage: &'static str,
    percent: u8,
) {
    if let Some(channel) = channel {
        send_progress(channel, stage, percent);
    }
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
            choose_images,
            choose_lut_directory,
            choose_output_path,
            choose_output_directory,
            available_output_paths,
            inspect_image,
            process_image,
            render_preview,
            render_source_preview
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{
        codecs::jpeg::JpegEncoder, ImageEncoder, Rgb, RgbImage, Rgba, Rgba32FImage, RgbaImage,
    };
    use std::time::UNIX_EPOCH;

    #[test]
    fn tone_maps_float_hdr_into_sdr_rgba8() {
        let source = Rgba32FImage::from_pixel(1, 1, Rgba([4.0, 4.0, 4.0, 0.5]));
        let output = convert_to_sdr(DynamicImage::ImageRgba32F(source), Some(ImageFormat::Hdr));
        let pixel = output.get_pixel(0, 0).0;
        assert!(pixel[0] > 240 && pixel[0] < 255);
        assert_eq!(pixel[0], pixel[1]);
        assert_eq!(pixel[1], pixel[2]);
        assert_eq!(pixel[3], 128);
    }

    #[test]
    fn normalizes_exif_orientation_while_decoding() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let input = std::env::temp_dir().join(format!(
            "luty-orientation-test-{}-{nonce}.jpg",
            std::process::id()
        ));
        let source = RgbImage::from_pixel(2, 3, Rgb([64, 128, 192]));
        let file = fs::File::create(&input).unwrap();
        let mut encoder = JpegEncoder::new_with_quality(BufWriter::new(file), 95);
        // Little-endian TIFF payload with EXIF orientation 6 (rotate 90° clockwise).
        encoder
            .set_exif_metadata(vec![
                0x49, 0x49, 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, 0x01, 0x00, 0x12, 0x01, 0x03, 0x00,
                0x01, 0x00, 0x00, 0x00, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ])
            .unwrap();
        encoder.encode_image(&source).unwrap();
        drop(encoder);

        let decoded = decode_image(&input).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (3, 2));
        let inspected = inspect_image(input.to_string_lossy().into_owned()).unwrap();
        assert_eq!((inspected.width, inspected.height), (3, 2));
        fs::remove_file(input).unwrap();
    }

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
        let preview = render_preview_blocking(
            &state,
            &input.to_string_lossy(),
            &cube.to_string_lossy(),
            1.0,
            &directory.join("previews"),
        )
        .unwrap();
        assert_eq!((preview.width, preview.height), (1, 1));
        let preview_pixel = image::open(preview.output_path)
            .unwrap()
            .into_rgba8()
            .get_pixel(0, 0)
            .0;
        assert_eq!(preview_pixel, [0, 255, 255, 137]);
        fs::remove_dir_all(directory).unwrap();
    }
}
