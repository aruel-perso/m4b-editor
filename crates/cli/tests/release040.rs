use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
fn fixture(name: &str) -> PathBuf {
    std::env::var_os("M4B_EDITOR_TEST_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures"))
        .join(name)
}
fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_m4b-editor-cli"))
        .args(args)
        .output()
        .unwrap()
}
fn ok(args: &[&str]) {
    let output = cli(args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn cli_portable_copy_stream_formats_preview_and_encrypted_import() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let project = source.path().join("book.m4bproj");
    let project = project.to_str().unwrap();
    ok(&[
        "new",
        "--project",
        project,
        "--title",
        "Livre",
        "--publisher",
        "Press",
        "--book-language",
        "fra",
        fixture("tone440.wav").to_str().unwrap(),
    ]);
    let value: serde_json::Value = serde_json::from_slice(&fs::read(project).unwrap()).unwrap();
    assert_eq!(value["settings"]["bitrate_kbps"], 160);
    let mut child = Command::new(env!("CARGO_BIN_EXE_m4b-editor-cli"))
        .args([
            "import",
            project,
            "--password-stdin",
            fixture("encrypted.zip").to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"fixture\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("fixture"));
    let portable = target.path().join("portable");
    ok(&["copy", project, portable.to_str().unwrap()]);
    drop(source);
    let project = portable.join("book.m4bproj");
    let project = project.to_str().unwrap();
    ok(&["check", project]);
    for format in ["m4b", "mp3", "flac"] {
        let output = target.path().join(format!("book.{format}"));
        let output = output.to_str().unwrap();
        ok(&["export", project, output, "--format", format]);
        assert!(!cli(&["export", project, output, "--format", format])
            .status
            .success());
        let preview = target.path().join(format!("preview.{format}"));
        ok(&[
            "preview",
            project,
            preview.to_str().unwrap(),
            "--format",
            format,
            "--start-ms",
            "150",
            "--duration-ms",
            "200",
        ]);
    }
    let stream_project = target.path().join("streams.m4bproj");
    let stream_project = stream_project.to_str().unwrap();
    ok(&[
        "new",
        "--project",
        stream_project,
        "--stream",
        "1",
        fixture("multistream.mka").to_str().unwrap(),
    ]);
    ok(&["stream", stream_project, "0", "0"]);
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(stream_project).unwrap()).unwrap();
    assert_eq!(value["tracks"][0]["stream_index"], 0);
    assert_eq!(value["tracks"][0]["sample_rate"], 8000);
    let probe = cli(&["probe", fixture("multistream.mka").to_str().unwrap()]);
    assert!(probe.status.success());
    let value: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    assert_eq!(value["audio_streams"].as_array().unwrap().len(), 2);
}
