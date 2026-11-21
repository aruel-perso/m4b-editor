use m4b_editor_core::*;
use std::{
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
fn synth(path: &Path, frequency: u32, rate: u32) {
    assert!(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency={frequency}:duration=1:sample_rate={rate}"),
            "-c:a",
            "pcm_s16le"
        ])
        .arg(path)
        .status()
        .unwrap()
        .success());
}
#[test]
fn real_export_roundtrip_order_cover_and_no_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let b = DesktopFfmpeg::default();
    let a = dir.path().join("track2.wav");
    let z = dir.path().join("track10.wav");
    synth(&a, 440, 44100);
    synth(&z, 880, 32000);
    // Dependency-free tiny PPM cover, decoded and converted by FFmpeg.
    let cover = dir.path().join("cover.ppm");
    std::fs::write(&cover, b"P6\n2 2\n255\n\xff\0\0\xff\0\0\xff\0\0\xff\0\0").unwrap();
    let mut p = Project::default();
    p.import(&b, vec![z, a.clone()]).unwrap();
    assert_eq!(
        dunce::canonicalize(&p.tracks[0].path).unwrap(),
        dunce::canonicalize(a).unwrap()
    );
    p.metadata = Metadata {
        title: "Livre = # ; é".into(),
        author: "Author".into(),
        narrator: "Narrator".into(),
        description: "First line\nSecond line".into(),
        genre: "Audiobook".into(),
        date: "2025".into(),
        copyright: "© Author".into(),
        publisher: String::new(),
        book_language: String::new(),
    };
    p.cover = Some(cover);
    let project = dir.path().join("book.json");
    p.save(&project).unwrap();
    let p = Project::load(&project).unwrap();
    let output = dir.path().join("book.m4b");
    let cancel = Arc::new(AtomicBool::new(false));
    let mut progress = 0.;
    b.export(&p, &output, cancel, &mut |e| {
        if let ExportEvent::Progress(v) = e {
            progress = v;
        }
    })
    .unwrap();
    assert_eq!(progress, 1.);
    let probe = b.probe(&output).unwrap();
    assert_eq!(probe.track.codec, "aac");
    assert!(probe.track.duration_ms.abs_diff(2000) < 100);
    assert_eq!(probe.chapters.len(), 2);
    assert_eq!(probe.chapters[1].start_ms, 1000);
    assert!(probe.has_cover);
    assert_eq!(probe.metadata.title, p.metadata.title);
    let reopened = b.open(&output).unwrap();
    assert_eq!(reopened.cover.as_deref(), Some(output.as_path()));
    assert_eq!(reopened.chapters.len(), p.chapters.len());
    assert_eq!(reopened.metadata.narrator, p.metadata.narrator);
    // A new default bitrate must not re-encode an existing AAC audiobook.
    for (i, source) in [&output, p.cover.as_ref().unwrap()].iter().enumerate() {
        let preview = dir.path().join(format!("preview-{i}.png"));
        b.extract_cover_preview(source, &preview).unwrap();
        let bytes = std::fs::read(preview).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()) <= 560);
        assert!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()) <= 560);
    }
    let mut remux = reopened.clone();
    remux.settings.bitrate_kbps = 320;
    remux.metadata.title = "Remuxed title".into();
    assert!(remux.can_remux());
    let remux_output = dir.path().join("remuxed.m4b");
    let mut remux_log = false;
    b.export(
        &remux,
        &remux_output,
        Arc::new(AtomicBool::new(false)),
        &mut |event| {
            if let ExportEvent::Log(message) = event {
                remux_log |= message.contains("Lossless AAC remux");
            }
        },
    )
    .unwrap();
    assert!(remux_log);
    let packets = |path: &std::path::Path| {
        let output = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-show_packets",
                "-show_data_hash",
                "sha256",
                "-show_entries",
                "packet=data_hash",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let packets: Vec<String> = result["packets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|packet| packet["data_hash"].as_str().unwrap().to_owned())
            .collect();
        assert!(!packets.is_empty());
        packets
    };
    assert_eq!(packets(&output), packets(&remux_output));
    assert_eq!(
        b.probe(&remux_output).unwrap().metadata.title,
        "Remuxed title"
    );
    remux.settings.lossless_remux = false;
    assert!(!remux.can_remux());
    let mut imported = Project::default();
    imported.import(&b, vec![output.clone()]).unwrap();
    assert_eq!(imported.metadata.genre, p.metadata.genre);
    assert_eq!(imported.chapters.len(), p.chapters.len());
    assert!(imported.cover.is_some());
    let import_source = dir.path().join("import-source.m4b");
    std::fs::copy(&output, &import_source).unwrap();
    let mut owned = Project::named("Imported").unwrap();
    owned
        .import_assets(
            &b,
            vec![import_source.clone()],
            &dir.path().join("owned.assets"),
        )
        .unwrap();
    std::fs::remove_file(import_source).unwrap();
    assert!(owned.cover.as_ref().unwrap().is_file());
    assert_eq!(owned.metadata.copyright, p.metadata.copyright);
    let owned_output = dir.path().join("owned.m4b");
    b.export(
        &owned,
        &owned_output,
        Arc::new(AtomicBool::new(false)),
        &mut |_| {},
    )
    .unwrap();
    // Decode small windows and count zero crossings: validates audible concatenation order.
    for (offset, expected) in [("0.2", 440.), ("1.2", 880.)] {
        let raw = Command::new("ffmpeg")
            .args(["-v", "error", "-ss", offset, "-i"])
            .arg(&output)
            .args([
                "-t", "0.5", "-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "s16le", "-",
            ])
            .output()
            .unwrap();
        assert!(raw.status.success());
        let samples: Vec<i16> = raw
            .stdout
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        let crossings = samples.windows(2).filter(|w| w[0] <= 0 && w[1] > 0).count() as f64;
        let hz = crossings * 48000. / samples.len() as f64;
        assert!((hz - expected).abs() < 12., "{hz} != {expected}");
    }
    let before = std::fs::read(&output).unwrap();
    assert!(b
        .export(&p, &output, Arc::new(AtomicBool::new(false)), &mut |_| {})
        .is_err());
    assert_eq!(std::fs::read(&output).unwrap(), before);
    let cancelled = dir.path().join("cancelled.m4b");
    assert!(b
        .export(&p, &cancelled, Arc::new(AtomicBool::new(true)), &mut |_| {})
        .is_err());
    assert!(!cancelled.exists());
    let active = dir.path().join("active-cancel.m4b");
    let token = Arc::new(AtomicBool::new(false));
    let signal = token.clone();
    assert!(b
        .export(
            &p,
            &active,
            token,
            &mut |event| if matches!(event,ExportEvent::Progress(v) if
    v>0.)
            {
                signal.store(true, Ordering::Relaxed)
            }
        )
        .is_err());
    assert!(!active.exists());
    let race = dir.path().join("race.m4b");
    assert!(b
        .export(
            &p,
            &race,
            Arc::new(AtomicBool::new(false)),
            &mut |event| if matches!(event,ExportEvent::Log(ref s) if
    s=="Encoding AAC and chapters")
            {
                std::fs::write(&race, b"other writer").unwrap();
            }
        )
        .is_err());
    assert_eq!(std::fs::read(&race).unwrap(), b"other writer");
    let invalid = dir.path().join("broken.png");
    std::fs::write(&invalid, b"not an image").unwrap();
    let mut broken = p.clone();
    broken.cover = Some(invalid);
    let failed = dir.path().join("failed.m4b");
    assert!(b
        .export(
            &broken,
            &failed,
            Arc::new(AtomicBool::new(false)),
            &mut |_| {}
        )
        .is_err());
    assert!(!failed.exists());
    assert!(!std::fs::read_dir(dir.path()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".m4b-editor-")));
}

#[test]
fn mp3_padding_does_not_shift_following_chapters() {
    let dir = tempfile::tempdir().unwrap();
    let mp3 = dir.path().join("track1.mp3");
    let wav = dir.path().join("track2.wav");
    // Genuine one-second MP3 generated by FFmpeg/libmp3lame; input decoding must
    // not require shipping an MP3 encoder in the Windows runtime.
    std::fs::write(&mp3, include_bytes!("fixtures/one-second-tone.mp3")).unwrap();
    synth(&wav, 880, 44100);
    let b = DesktopFfmpeg::default();
    let mut p = Project::default();
    p.import(&b, vec![wav, mp3]).unwrap();
    // FFmpeg versions differ in whether header duration includes MP3 padding.
    assert!((900..=1200).contains(&p.chapters[1].start_ms));
    let out = dir.path().join("gapless.m4b");
    b.export(&p, &out, Arc::new(AtomicBool::new(false)), &mut |_| {})
        .unwrap();
    let probe = b.probe(&out).unwrap();
    assert_eq!(probe.chapters[1].start_ms, 1000);
    assert!(probe.track.duration_ms.abs_diff(2000) < 100);
}

/// Opt-in acceptance for a complete multi-track transcode, full output decode,
/// and cancellation during a running encoder with an isolated destination.
#[test]
#[ignore = "long-form synthetic audio workflow; needs FFmpeg and 450 MiB free"]
fn long_form_export_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let backend = DesktopFfmpeg::default();
    let mut inputs = Vec::new();
    for i in [10, 2, 1] {
        let path = dir.path().join(format!("track{i}.flac"));
        assert!(Command::new(&backend.ffmpeg)
            .args(["-v", "error", "-nostdin", "-f", "lavfi", "-i"])
            .arg(format!(
                "sine=frequency={}:duration=300:sample_rate=12000",
                400 + i
            ))
            .arg(&path)
            .status()
            .unwrap()
            .success());
        inputs.push(path);
    }
    let mut project = Project::default();
    project.import(&backend, inputs).unwrap();
    assert_eq!(project.duration_ms(), 900_000);
    assert!(project.tracks[1].path.ends_with("track2.flac"));
    let output = dir.path().join("long.m4b");
    let mut values = Vec::new();
    let mut verified = false;
    backend
        .export(
            &project,
            &output,
            Arc::new(AtomicBool::new(false)),
            &mut |event| match event {
                ExportEvent::Progress(value) => values.push(value),
                ExportEvent::Phase(ExportPhase::VerifyingAudio) => verified = true,
                _ => {}
            },
        )
        .unwrap();
    assert!(verified);
    assert!(values
        .windows(2)
        .all(|pair| pair[1] + f32::EPSILON >= pair[0]));
    assert!(values.iter().any(|v| (0.45..0.95).contains(v)));
    assert_eq!(values.last(), Some(&1.));
    let output = backend.probe(&output).unwrap();
    assert_eq!(output.track.codec, "aac");
    assert!(output.track.duration_ms.abs_diff(900_000) < 100);
    assert_eq!(
        output
            .chapters
            .iter()
            .map(|c| c.start_ms)
            .collect::<Vec<_>>(),
        [0, 300_000, 600_000]
    );

    let cancelled = dir.path().join("cancelled.m4b");
    let cancel = Arc::new(AtomicBool::new(false));
    let error = backend
        .export(&project, &cancelled, cancel.clone(), &mut |event| {
            if matches!(event, ExportEvent::Progress(value) if value > 0.) {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    assert!(!cancelled.exists());
    assert!(!std::fs::read_dir(dir.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".m4b-editor-")));
}
