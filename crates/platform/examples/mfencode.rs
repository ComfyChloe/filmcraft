//! Diagnostics: does this machine's Media Foundation hardware encoder take an export, and what
//! does it write? `mfencode [--format h264|hevc] [--width W] [--height H] [--fps N]
//! [--frames N] [--bitrate K] [--profile baseline|main|high] [--cbr]`
//!
//! The Media Foundation encoder is driven **directly**, so this measures that path and not
//! whichever encoder the export registry happens to try first: on a machine that also has NVENC
//! the registry prefers NVENC, so an end-to-end export would never reach Media Foundation.
//!
//! Nothing is written to disk: the encoder's access units are counted and sized.

use std::time::Instant;

use filmcraft_export::{BitrateMode, EncoderFrame, ExportSettings, Format, H264Profile, HardwareEncoding};
use filmcraft_time::FrameRate;

/// A logger that prints what the encoder says about itself (which MFT it opened).
struct Stderr;

impl log::Log for Stderr {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, r: &log::Record<'_>) {
        if r.level() <= log::Level::Debug {
            eprintln!("[{}] {}", r.level(), r.args());
        }
    }
    fn flush(&self) {}
}

static LOGGER: Stderr = Stderr;

fn main() {
    let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(log::LevelFilter::Info));

    let mut format = Format::H264;
    let mut width = 640u32;
    let mut height = 360u32;
    let mut fps = 30u32;
    let mut frames = 60u32;
    let mut bitrate = 4000u32;
    let mut profile = H264Profile::High;
    let mut mode = BitrateMode::Vbr1Pass;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].clone();
        i += 1;
        let mut value = || {
            let v = args.get(i).cloned().unwrap_or_default();
            i += 1;
            v
        };
        match a.as_str() {
            "--format" => {
                format = match value().as_str() {
                    "hevc" | "h265" | "Hevc" => Format::Hevc,
                    "h264" | "H264" => Format::H264,
                    other => {
                        eprintln!("--format: unknown format {other}");
                        std::process::exit(2);
                    }
                };
            }
            "--width" => width = value().parse().unwrap_or(width),
            "--height" => height = value().parse().unwrap_or(height),
            "--fps" => fps = value().parse().unwrap_or(fps),
            "--frames" => frames = value().parse().unwrap_or(frames),
            "--bitrate" => bitrate = value().parse().unwrap_or(bitrate),
            "--profile" => {
                profile = match value().as_str() {
                    "baseline" | "Baseline" => H264Profile::Baseline,
                    "main" | "Main" => H264Profile::Main,
                    "high" | "High" => H264Profile::High,
                    other => {
                        eprintln!("--profile: unknown profile {other}");
                        std::process::exit(2);
                    }
                };
            }
            "--cbr" => mode = BitrateMode::Cbr,
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }

    let rate = FrameRate { num: i64::from(fps.max(1)), den: 1 };
    let settings = ExportSettings {
        format,
        bitrate_kbps: bitrate,
        hardware_encoding: HardwareEncoding::Auto,
        h264_profile: profile,
        bitrate_mode: mode,
        ..Default::default()
    };

    println!("request: {} {width}x{height} @ {fps} fps, {bitrate} kbps, profile {}, mode {}", format_label(format), profile.label(), mode.label());

    let mut encoder = match filmcraft_platform::media_foundation::encode::factory(format, width, height, rate, &settings) {
        Some(Ok(e)) => e,
        Some(Err(e)) => {
            println!("declined: {e}");
            std::process::exit(2);
        }
        None => {
            println!("declined: no Media Foundation encoder took this request");
            std::process::exit(3);
        }
    };

    // A moving pattern: a flat picture would let an encoder drop everything but the first frame.
    let mut rgba = vec![0u8; (width as usize) * (height as usize) * 4];
    let mut packets = 0u64;
    let mut keys = 0u64;
    let mut bytes = 0u64;
    let t0 = Instant::now();
    for index in 0..frames {
        for y in 0..height as usize {
            for x in 0..width as usize {
                let o = (y * width as usize + x) * 4;
                rgba[o] = (x as u32 + index * 4) as u8;
                rgba[o + 1] = (y as u32 + index * 2) as u8;
                rgba[o + 2] = ((x + y) as u32 + index * 7) as u8;
                rgba[o + 3] = 255;
            }
        }
        let frame = EncoderFrame { width, height, rgba: &rgba, hdr: None, index: u64::from(index) };
        match encoder.encode(&frame) {
            Ok(out) => {
                for p in &out {
                    packets += 1;
                    bytes += p.data.len() as u64;
                    if p.key {
                        keys += 1;
                    }
                }
            }
            Err(e) => {
                println!("failed at picture {index}: {e}");
                std::process::exit(4);
            }
        }
    }
    match encoder.flush() {
        Ok(out) => {
            for p in &out {
                packets += 1;
                bytes += p.data.len() as u64;
                if p.key {
                    keys += 1;
                }
            }
        }
        Err(e) => {
            println!("flush failed: {e}");
            std::process::exit(5);
        }
    }
    let elapsed = t0.elapsed();
    let secs = elapsed.as_secs_f64().max(0.001);
    // The stream's bit rate is what the file carries, not how fast the encoder ran: bytes over the
    // length of the encoded material.
    let seconds = f64::from(frames) / f64::from(fps.max(1));
    let kbps = bytes as f64 * 8.0 / seconds / 1000.0;
    println!("result: {packets} access units ({keys} key), {bytes} bytes, {seconds:.2}s of material");
    println!("bitrate: {kbps:.0} kbps against a {bitrate} kbps target ({:.2}x)", kbps / f64::from(bitrate));
    println!("speed: {packets} pictures in {secs:.2}s ({:.1} fps)", packets as f64 / secs);
    println!("sample entry: {}", describe_entry(&encoder.sample_entry()));
    if packets == 0 {
        println!("result: the encoder produced nothing");
        std::process::exit(6);
    }
}

/// The profile the encoder actually wrote, read back from the sample entry (not from the request).
fn describe_entry(entry: &filmcraft_isobmff::SampleEntry) -> String {
    match &entry.codec {
        filmcraft_isobmff::CodecConfig::Avc(c) => {
            format!("avcC: profile_idc {} (compatibility {}, level {}), {} SPS, {} PPS", c.profile, c.compatibility, c.level, c.sps.len(), c.pps.len())
        }
        filmcraft_isobmff::CodecConfig::Hevc(c) => {
            format!("hvcC: general_profile_idc {}, general_level_idc {}, {}-bit", c.general_profile_idc, c.general_level_idc, c.bit_depth_luma)
        }
        other => format!("{other:?}"),
    }
}

fn format_label(f: Format) -> &'static str {
    match f {
        Format::H264 => "H.264",
        Format::Hevc => "H.265",
        _ => "other",
    }
}
