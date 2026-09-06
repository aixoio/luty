use serde::Serialize;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use walkdir::WalkDir;

#[derive(Clone, Debug)]
pub struct CubeLut {
    pub size: usize,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    values: Vec<[f32; 3]>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LutDescriptor {
    pub path: String,
    pub name: String,
    pub title: Option<String>,
    pub size: Option<usize>,
    pub file_size: u64,
    pub modified_at_ms: Option<u128>,
    pub supported: bool,
    pub error: Option<String>,
}

impl CubeLut {
    pub fn parse(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|error| format!("Could not read LUT '{}': {error}", path.display()))?;
        Self::parse_str(&text)
    }

    fn parse_str(text: &str) -> Result<Self, String> {
        let mut size = None;
        let mut domain_min = [0.0; 3];
        let mut domain_max = [1.0; 3];
        let mut values = Vec::new();

        for (line_number, raw_line) in text.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split_whitespace();
            let first = fields.next().unwrap_or_default();
            match first {
                "TITLE" => continue,
                "LUT_3D_SIZE" => {
                    let parsed = fields
                        .next()
                        .ok_or_else(|| {
                            format!("Missing LUT_3D_SIZE value on line {}", line_number + 1)
                        })?
                        .parse::<usize>()
                        .map_err(|_| format!("Invalid LUT_3D_SIZE on line {}", line_number + 1))?;
                    if !(2..=256).contains(&parsed) {
                        return Err(format!(
                            "LUT_3D_SIZE must be between 2 and 256, got {parsed}"
                        ));
                    }
                    size = Some(parsed);
                }
                "LUT_1D_SIZE" => return Err("1D .cube LUTs are not supported yet".into()),
                "DOMAIN_MIN" => domain_min = parse_triplet(fields, line_number + 1)?,
                "DOMAIN_MAX" => domain_max = parse_triplet(fields, line_number + 1)?,
                _ => {
                    if first.parse::<f32>().is_ok() {
                        let mut numbers = line.split_whitespace().map(str::parse::<f32>);
                        let value = [
                            parse_number(numbers.next(), line_number + 1)?,
                            parse_number(numbers.next(), line_number + 1)?,
                            parse_number(numbers.next(), line_number + 1)?,
                        ];
                        if value.iter().any(|component| !component.is_finite()) {
                            return Err(format!(
                                "Non-finite LUT value on line {}",
                                line_number + 1
                            ));
                        }
                        values.push(value);
                    }
                }
            }
        }

        let size = size.ok_or_else(|| "Missing LUT_3D_SIZE".to_string())?;
        let expected = size
            .checked_mul(size)
            .and_then(|count| count.checked_mul(size))
            .ok_or_else(|| "LUT size is too large".to_string())?;
        if values.len() != expected {
            return Err(format!(
                "Expected {expected} LUT entries, found {}",
                values.len()
            ));
        }
        for channel in 0..3 {
            if domain_max[channel] <= domain_min[channel] {
                return Err("Each DOMAIN_MAX component must be greater than DOMAIN_MIN".into());
            }
        }

        Ok(Self {
            size,
            domain_min,
            domain_max,
            values,
        })
    }

    #[inline]
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        let max_index = (self.size - 1) as f32;
        let mut position = [0.0; 3];
        for channel in 0..3 {
            position[channel] = ((rgb[channel] - self.domain_min[channel])
                / (self.domain_max[channel] - self.domain_min[channel]))
                .clamp(0.0, 1.0)
                * max_index;
        }

        let lower = [
            position[0].floor() as usize,
            position[1].floor() as usize,
            position[2].floor() as usize,
        ];
        let upper = [
            (lower[0] + 1).min(self.size - 1),
            (lower[1] + 1).min(self.size - 1),
            (lower[2] + 1).min(self.size - 1),
        ];
        let fraction = [
            position[0] - lower[0] as f32,
            position[1] - lower[1] as f32,
            position[2] - lower[2] as f32,
        ];

        // The .cube format stores red as the fastest-changing coordinate.
        let c000 = self.at(lower[0], lower[1], lower[2]);
        let c100 = self.at(upper[0], lower[1], lower[2]);
        let c010 = self.at(lower[0], upper[1], lower[2]);
        let c110 = self.at(upper[0], upper[1], lower[2]);
        let c001 = self.at(lower[0], lower[1], upper[2]);
        let c101 = self.at(upper[0], lower[1], upper[2]);
        let c011 = self.at(lower[0], upper[1], upper[2]);
        let c111 = self.at(upper[0], upper[1], upper[2]);

        let mut result = [0.0; 3];
        for channel in 0..3 {
            let x00 = lerp(c000[channel], c100[channel], fraction[0]);
            let x10 = lerp(c010[channel], c110[channel], fraction[0]);
            let x01 = lerp(c001[channel], c101[channel], fraction[0]);
            let x11 = lerp(c011[channel], c111[channel], fraction[0]);
            result[channel] = lerp(
                lerp(x00, x10, fraction[1]),
                lerp(x01, x11, fraction[1]),
                fraction[2],
            );
        }
        result
    }

    #[inline]
    fn at(&self, red: usize, green: usize, blue: usize) -> [f32; 3] {
        self.values[red + green * self.size + blue * self.size * self.size]
    }
}

pub fn scan_directory(directory: &Path) -> Result<Vec<LutDescriptor>, String> {
    if !directory.is_dir() {
        return Err(format!("'{}' is not a directory", directory.display()));
    }

    let mut files = WalkDir::new(directory)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("cube"))
        })
        .map(|entry| describe(entry.into_path()))
        .collect::<Vec<_>>();
    files.sort_by_key(|descriptor| descriptor.name.to_lowercase());
    Ok(files)
}

fn describe(path: PathBuf) -> LutDescriptor {
    let metadata = fs::metadata(&path).ok();
    let modified_at_ms = metadata
        .as_ref()
        .and_then(|value| value.modified().ok())
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis());
    let name = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let (title, size, error) = read_metadata(&path);
    LutDescriptor {
        path: path.to_string_lossy().into_owned(),
        name,
        title,
        size,
        file_size: metadata.map_or(0, |value| value.len()),
        modified_at_ms,
        supported: error.is_none(),
        error,
    }
}

fn read_metadata(path: &Path) -> (Option<String>, Option<usize>, Option<String>) {
    let Ok(file) = fs::File::open(path) else {
        return (None, None, Some("Could not read LUT".into()));
    };
    let mut title = None;
    for line in BufReader::new(file).lines().take(128) {
        let Ok(line) = line else {
            return (title, None, Some("Could not read LUT header".into()));
        };
        let line = line.trim();
        if let Some(value) = line.strip_prefix("TITLE") {
            title = Some(value.trim().trim_matches('"').to_owned());
        } else if line.starts_with("LUT_1D_SIZE") {
            return (
                title,
                None,
                Some("1D .cube LUTs are not supported yet".into()),
            );
        } else if let Some(value) = line.strip_prefix("LUT_3D_SIZE") {
            return match value.trim().parse::<usize>() {
                Ok(size) if (2..=256).contains(&size) => (title, Some(size), None),
                Ok(size) => (
                    title,
                    None,
                    Some(format!("LUT_3D_SIZE must be between 2 and 256, got {size}")),
                ),
                Err(_) => (title, None, Some("Invalid LUT_3D_SIZE".into())),
            };
        }
    }
    (
        title,
        None,
        Some("Missing LUT_3D_SIZE near the start of the file".into()),
    )
}

fn parse_triplet<'a>(
    mut fields: impl Iterator<Item = &'a str>,
    line: usize,
) -> Result<[f32; 3], String> {
    Ok([
        parse_number(fields.next().map(str::parse), line)?,
        parse_number(fields.next().map(str::parse), line)?,
        parse_number(fields.next().map(str::parse), line)?,
    ])
}

fn parse_number(
    value: Option<Result<f32, std::num::ParseFloatError>>,
    line: usize,
) -> Result<f32, String> {
    value
        .ok_or_else(|| format!("Missing numeric component on line {line}"))?
        .map_err(|_| format!("Invalid number on line {line}"))
}

#[inline]
fn lerp(start: f32, end: f32, amount: f32) -> f32 {
    start + (end - start) * amount
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_interpolates_identity_cube() {
        let lut = CubeLut::parse_str(
            "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n",
        )
        .unwrap();
        let sampled = lut.sample([0.25, 0.5, 0.75]);
        assert!((sampled[0] - 0.25).abs() < 0.0001);
        assert!((sampled[1] - 0.5).abs() < 0.0001);
        assert!((sampled[2] - 0.75).abs() < 0.0001);
    }

    #[test]
    fn rejects_incomplete_cube() {
        assert!(CubeLut::parse_str("LUT_3D_SIZE 2\n0 0 0\n").is_err());
    }
}
