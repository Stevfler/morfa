//! ffmpeg command lines: codecs, quality, resize, metadata.
use super::super::{routes, ConvertOpts, Resize};
use std::path::Path;

/// Quality 1..100 -> CRF, linear from `lo` (q=1) to `hi` (q=100).
fn crf(q: u8, lo: u32, hi: u32) -> u32 {
    let q = q.clamp(1, 100) as u32 - 1;
    lo - ((lo - hi) * q + 49) / 99
}

/// x264 CRF: 100 -> 16 … 1 -> 36.
pub fn crf_x264(q: u8) -> u32 {
    crf(q, 36, 16)
}

/// VP9 CRF: 100 -> 20 … 1 -> 45.
pub fn crf_vp9(q: u8) -> u32 {
    crf(q, 45, 20)
}

/// Lossy audio bitrate in kbit/s; opus needs ~0.6x for the same quality.
pub fn audio_kbps(q: u8, opus: bool) -> u32 {
    let b = match q {
        90.. => 320,
        75..=89 => 192,
        55..=74 => 128,
        _ => 96,
    };
    if opus { b * 3 / 5 } else { b }
}

/// Scale filter for the resize option (even dims, "max" never upscales).
pub fn scale(r: &Resize) -> String {
    if r.mode == "percent" {
        return format!("scale=trunc(iw*{}/200)*2:-2", r.value.min(100));
    }
    let n = r.value;
    format!("scale='if(gt(iw,ih),trunc(min({n},iw)/2)*2,-2)':'if(gt(iw,ih),-2,trunc(min({n},ih)/2)*2)'")
}

fn audio_codec(fmt: &str, q: u8) -> Vec<String> {
    let (c, br) = match fmt {
        "mp3" => ("libmp3lame", Some(audio_kbps(q, false))),
        "m4a" | "mp4" | "mov" | "mkv" => ("aac", Some(audio_kbps(q, false))),
        "ogg" => ("libvorbis", Some(audio_kbps(q, false))),
        "opus" | "webm" => ("libopus", Some(audio_kbps(q, true))),
        "flac" => ("flac", None),
        _ => ("pcm_s16le", None), // wav
    };
    let mut v = vec!["-c:a".into(), c.into()];
    if let Some(b) = br {
        v.extend(["-b:a".into(), format!("{b}k")]);
    }
    v
}

/// Full argument list: `-hide_banner -nostdin -y -i src … -progress pipe:1 -nostats tmp`.
pub fn build(src: &Path, tmp: &Path, o: &ConvertOpts) -> Vec<String> {
    let fmt = o.format.as_str();
    let q = o.quality.clamp(1, 100);
    let mut a: Vec<String> = ["-hide_banner", "-nostdin", "-y", "-i"].map(String::from).into();
    a.push(src.to_string_lossy().into());
    let size = o.resize.as_ref().filter(|r| r.value > 0).map(scale);
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    if routes::is_audio(fmt) {
        a.push("-vn".into());
        a.extend(audio_codec(fmt, q));
    } else if fmt == "gif" {
        let mut f = vec!["fps=15".to_string()];
        f.extend(size);
        f.push("split[a][b];[a]palettegen[p];[b][p]paletteuse".into());
        a.extend(["-vf".into(), f.join(","), "-an".into(), "-loop".into(), "0".into()]);
    } else if fmt == "webm" {
        a.extend(s(&["-c:v", "libvpx-vp9", "-b:v", "0", "-crf"]));
        a.push(crf_vp9(q).to_string());
        a.extend(s(&["-row-mt", "1", "-pix_fmt", "yuv420p"]));
        if let Some(f) = size {
            a.extend(["-vf".into(), f]);
        }
        a.extend(audio_codec(fmt, q));
    } else {
        // x264 + yuv420p needs even dimensions.
        let f = size.unwrap_or_else(|| "scale=trunc(iw/2)*2:trunc(ih/2)*2".into());
        a.extend(s(&["-c:v", "libx264", "-preset", "medium", "-crf"]));
        a.push(crf_x264(q).to_string());
        a.extend(s(&["-pix_fmt", "yuv420p", "-vf"]));
        a.push(f);
        if matches!(fmt, "mp4" | "mov") {
            a.extend(s(&["-movflags", "+faststart"]));
        }
        a.extend(audio_codec(fmt, q));
    }
    if o.strip_metadata {
        a.extend(s(&["-map_metadata", "-1", "-map_chapters", "-1"]));
    }
    a.extend(s(&["-progress", "pipe:1", "-nostats"]));
    a.push(tmp.to_string_lossy().into());
    a
}
