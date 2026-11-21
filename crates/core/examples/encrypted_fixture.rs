//! Regenerate the redistributable AES ZIP fixture from the synthetic WAV.
use std::{fs::File, io, path::PathBuf};
use zip::{write::SimpleFileOptions, AesMode, CompressionMethod, ZipWriter};

fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut audio = File::open(root.join("tone440.wav"))?;
    let mut zip = ZipWriter::new(File::create(root.join("encrypted.zip"))?);
    zip.start_file(
        "tone440.wav",
        SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .with_aes_encryption(AesMode::Aes256, "fixture"),
    )?;
    io::copy(&mut audio, &mut zip)?;
    zip.finish()?;
    Ok(())
}
