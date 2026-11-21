use m4b_editor_core::*;
use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
fn fixture(name: &str) -> PathBuf {
    std::env::var_os("M4B_EDITOR_TEST_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures"))
        .join(name)
}
fn cancel() -> Cancel {
    Arc::new(AtomicBool::new(false))
}
#[test]
fn real_formats_metadata_preview_and_absolute_stream_selection() {
    let d = tempfile::tempdir().unwrap();
    let b = DesktopFfmpeg::default();
    let first = b
        .probe_stream(&fixture("multistream.mka"), Some(0))
        .unwrap();
    let second = b
        .probe_stream(&fixture("multistream.mka"), Some(1))
        .unwrap();
    assert_eq!(first.track.sample_rate, 8000);
    assert_eq!(second.track.sample_rate, 16000);
    assert!(b
        .probe_stream(&fixture("multistream.mka"), Some(2))
        .is_err());
    let mut p = Project {
        tracks: vec![second.track],
        ..Project::default()
    };
    p.reset_chapters();
    p.chapters.push(Chapter {
        start_ms: 100,
        title: "Personnalisé".into(),
    });
    p.metadata.title = "Livre é".into();
    p.metadata.author = "Auteur".into();
    p.metadata.narrator = "Narrateur".into();
    p.metadata.description = "Description\nDeuxième ligne".into();
    p.metadata.genre = "Audiobook".into();
    p.metadata.date = "2026".into();
    p.metadata.copyright = "© Auteur".into();
    p.metadata.publisher = "Éditions du test".into();
    p.metadata.book_language = "fra".into();
    p.cover = Some(fixture("cover.ppm"));
    for format in [ExportFormat::M4b, ExportFormat::Mp3, ExportFormat::Flac] {
        p.settings.format = format;
        let output = d.path().join(format!("book.{}", format.extension()));
        b.export(&p, &output, cancel(), &mut |_| {}).unwrap();
        let probe = b.probe(&output).unwrap();
        assert_eq!(probe.track.codec, format.codec());
        assert_eq!(probe.metadata.publisher, p.metadata.publisher);
        assert_eq!(probe.chapters.len(), 2);
        assert_eq!(probe.chapters[1].start_ms, 100);
        assert_eq!(probe.chapters[1].title, "Personnalisé");
        if format == ExportFormat::M4b {
            assert_eq!(probe.metadata.book_language, "fra");
        }
        assert!(probe.has_cover);
        assert!(b.export(&p, &output, cancel(), &mut |_| {}).is_err());
        let preview = d.path().join(format!("preview.{}", format.extension()));
        b.encoded_preview(&p, &preview, 50, 150, cancel(), &mut |_| {})
            .unwrap();
        let probe = b.probe(&preview).unwrap();
        assert_eq!(probe.track.codec, format.codec());
        assert!(probe.track.duration_ms.abs_diff(150) < 200);
    }
}
#[test]
fn actual_encrypted_archives_zip64_and_cancelled_import_rollback() {
    let d = tempfile::tempdir().unwrap();
    let mut project = Project::default();
    let unsupported = d.path().join("audio.7z");
    std::fs::copy(fixture("audio.zip"), &unsupported).unwrap();
    assert!(project
        .import_assets(&DesktopFfmpeg::default(), vec![unsupported], d.path())
        .unwrap_err()
        .to_string()
        .contains("Unsupported import"));
    let disguised = d.path().join("disguised.rar");
    std::fs::copy(fixture("audio.zip"), &disguised).unwrap();
    assert!(project
        .import_assets(&DesktopFfmpeg::default(), vec![disguised], d.path())
        .unwrap_err()
        .to_string()
        .contains("Invalid RAR signature"));
    let b = DesktopFfmpeg::default();
    let d = tempfile::tempdir().unwrap();
    for name in ["encrypted.zip", "zip64.zip", "audio.rar"] {
        let mut p = Project::default();
        let assets = d.path().join(name);
        p.import_assets_with(
            &b,
            vec![fixture(name)],
            &assets,
            &Job::default(),
            name.starts_with("encrypted").then_some("fixture"),
        )
        .unwrap();
        assert_eq!(p.tracks.len(), 1);
        assert!(p.tracks[0].path.is_file());
        let before = serde_json::to_string(&p).unwrap();
        assert!(p
            .import_assets_with(
                &b,
                vec![fixture("encrypted.zip")],
                &assets,
                &Job::default(),
                Some("wrong")
            )
            .is_err());
        assert_eq!(serde_json::to_string(&p).unwrap(), before);
        let job = Job::new(Arc::new(AtomicBool::new(true)), Duration::from_secs(1));
        assert!(p
            .import_assets_with(&b, vec![fixture("audio.rar")], &assets, &job, None)
            .is_err());
    }
}
#[test]
fn adding_tracks_preserves_custom_chapters_and_rejects_ambiguous_timeline() {
    let b = DesktopFfmpeg::default();
    let mut p = Project::default();
    p.import(&b, vec![fixture("tone440.wav")]).unwrap();
    p.chapters.push(Chapter {
        start_ms: 100,
        title: "Custom".into(),
    });
    p.import(&b, vec![fixture("tone880.wav")]).unwrap();
    assert_eq!(p.chapters[1].title, "Custom");
    assert_eq!(p.chapters[1].start_ms, 100);
    p.chapters[1].start_ms = 9000;
    let before = serde_json::to_string(&p).unwrap();
    assert!(p.import(&b, vec![fixture("tone440.wav")]).is_err());
    assert_eq!(serde_json::to_string(&p).unwrap(), before);
}
#[test]
fn legacy_project_defaults_survive() {
    let project = Project::load(&fixture("legacy.json")).unwrap();
    project.validate().unwrap();
    assert_eq!(project.settings.bitrate_kbps, 96);
    assert_eq!(project.settings.format, ExportFormat::M4b);
    assert_eq!(project.tracks[0].stream_index, None);
    assert_eq!(project.chapters[1].title, "Personnalisé");
}
#[test]
fn frozen_fixture_manifest_hashes_match_actual_files() {
    use sha2::{Digest, Sha256};
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture("manifest.json")).unwrap()).unwrap();
    for entry in manifest["files"].as_array().unwrap() {
        let data = std::fs::read(fixture(entry["path"].as_str().unwrap())).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(data)),
            entry["sha256"].as_str().unwrap(),
            "{}",
            entry["path"]
        );
        assert!(entry["expected"].is_string());
    }
}
#[cfg(unix)]
#[test]
fn probe_deadline_and_midflight_storage_failure_reap_and_rollback() {
    use std::{fs, os::unix::fs::PermissionsExt, time::Instant};
    let root = tempfile::tempdir().unwrap();
    let fake = root.path().join("tool");
    let pid = root.path().join("pid");
    fs::write(
        &fake,
        format!("#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n", pid.display()),
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();
    let mut backend = DesktopFfmpeg {
        ffprobe: fake.clone(),
        job: Job::new(cancel(), Duration::from_millis(100)),
        ..DesktopFfmpeg::default()
    };
    let start = Instant::now();
    assert!(backend.probe(&fixture("tone440.wav")).is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
    #[cfg(target_os = "linux")]
    assert!(!Path::new("/proc")
        .join(fs::read_to_string(&pid).unwrap().trim())
        .exists());
    let real = DesktopFfmpeg::default();
    let mut p = Project::default();
    p.import(&real, vec![fixture("tone440.wav")]).unwrap();
    fs::write(&fake,"#!/bin/sh\nfor arg do\nif [ \"$arg\" = -encoders ]; then printf ' A..... aac test\\n'; exit 0; fi\ndone\nprintf 'No space left on device\\n' >&2\nexit 1\n").unwrap();
    backend = DesktopFfmpeg {
        ffmpeg: fake,
        ..DesktopFfmpeg::default()
    };
    let output = root.path().join("failed.m4b");
    let error = backend
        .export(&p, &output, cancel(), &mut |_| {})
        .unwrap_err();
    assert!(
        error.to_string().contains("No space left on device"),
        "{error:#}"
    );
    assert!(!output.exists());
    assert!(!fs::read_dir(root.path()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".m4b-editor-")));
}
