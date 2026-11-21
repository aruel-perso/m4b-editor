use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use m4b_editor_core::{DesktopFfmpeg, ExportEvent, MediaBackend, Project};
use std::{
    io::{self, BufRead, Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Parser)]
#[command(
    version,
    about = "M4B Editor: local audiobook creation; never overwrites exports"
)]
struct Args {
    #[arg(long)]
    ffmpeg: Option<PathBuf>,
    #[arg(long)]
    ffprobe: Option<PathBuf>,
    /// Maximum operation duration; Ctrl-C cancels every media operation.
    #[arg(long, default_value_t = 86400)]
    timeout_seconds: u64,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Import tracks in natural order and save a project (paths become absolute).
    New {
        #[arg(short, long)]
        project: PathBuf,
        #[arg(long, default_value = "")]
        title: String,
        #[arg(long, default_value = "")]
        author: String,
        #[arg(long, default_value = "")]
        narrator: String,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long)]
        cover: Option<PathBuf>,
        #[arg(long, default_value = "")]
        publisher: String,
        #[arg(long, default_value = "")]
        book_language: String,
        #[arg(long)]
        stream: Option<u32>,
        #[arg(long, default_value_t = 160)]
        bitrate: u32,
        #[arg(long, default_value = "m4b", value_parser = ["m4b", "mp3", "flac"])]
        format: String,
        /// Read an archive password from a pipe without putting it in argv.
        #[arg(long)]
        password_stdin: bool,
        #[arg(required = true)]
        tracks: Vec<PathBuf>,
    },

    /// Open an existing M4B and save its metadata/chapters as a project.
    Open {
        input: PathBuf,
        #[arg(short, long)]
        project: PathBuf,
        #[arg(long)]
        extract_cover: Option<PathBuf>,
    },
    Probe {
        input: PathBuf,
    },
    /// Copy a project with all media and cover into a new portable directory.
    Copy {
        project: PathBuf,
        destination: PathBuf,
    },
    /// Encode up to 30 seconds with the project output settings.
    Preview {
        project: PathBuf,
        output: PathBuf,
        #[arg(long, default_value_t = 0)]
        start_ms: u64,
        #[arg(long, default_value_t = 10000)]
        duration_ms: u64,
        #[arg(long, value_parser = ["m4b", "mp3", "flac"])]
        format: Option<String>,
    },
    /// Select an absolute audio stream index on one track.
    Stream {
        project: PathBuf,
        track: usize,
        index: u32,
    },
    /// Import audio/archives transactionally into an existing project.
    Import {
        project: PathBuf,
        #[arg(long)]
        password_stdin: bool,
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
    },
    Encoders,
    Check {
        project: PathBuf,
    },
    Export {
        project: PathBuf,
        output: PathBuf,
        #[arg(long, value_parser = ["m4b", "mp3", "flac"])]
        format: Option<String>,
    },

    /// Sequential newline JSON-RPC MCP server; stdout contains only protocol messages.
    Mcp,
}
fn main() -> Result<()> {
    let args = Args::parse();
    let mut backend = DesktopFfmpeg::default();
    if let Some(path) = args.ffmpeg {
        backend.ffmpeg = path;
    }
    if let Some(path) = args.ffprobe {
        backend.ffprobe = path;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    ctrlc::set_handler(move || c.store(true, Ordering::Relaxed))?;
    backend.job = m4b_editor_core::Job::new(
        cancel.clone(),
        std::time::Duration::from_secs(args.timeout_seconds),
    );
    match args.command {
        Action::New {
            project,
            title,
            author,
            narrator,
            description,
            cover,
            publisher,
            book_language,
            stream,
            bitrate,
            format,
            password_stdin,
            tracks,
        } => {
            anyhow::ensure!(!project.exists(), "Project exists");
            let mut p = Project::default();
            let password = read_password(password_stdin)?;
            p.import_assets_with(
                &backend,
                tracks,
                &project.with_extension("assets"),
                &backend.job,
                password.as_deref(),
            )?;
            if let Some(stream) = stream {
                for index in 0..p.tracks.len() {
                    p.select_stream(&backend, index, stream)?;
                }
            }
            for (input, field) in [
                (title, &mut p.metadata.title),
                (author, &mut p.metadata.author),
                (narrator, &mut p.metadata.narrator),
                (description, &mut p.metadata.description),
                (publisher, &mut p.metadata.publisher),
                (book_language, &mut p.metadata.book_language),
            ] {
                if !input.is_empty() {
                    *field = input;
                }
            }
            p.name = project
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            p.settings.bitrate_kbps = bitrate;
            p.settings.format = parse_format(&format);
            if let Some(cover) = cover {
                p.cover = Some(std::fs::canonicalize(cover)?);
            }
            p.validate()?;
            p.save_new(&project)?;
            println!(
                "Saved {} ({} tracks, {} ms)",
                project.display(),
                p.tracks.len(),
                p.duration_ms()
            );
        }
        Action::Open {
            input,
            project,
            extract_cover,
        } => {
            anyhow::ensure!(!project.exists(), "Project exists");
            let mut p = backend.open(&input)?;
            if let Some(cover) = extract_cover {
                backend.extract_cover(&input, &cover)?;
                p.cover = Some(std::fs::canonicalize(cover)?);
            }
            p.save_new(&project)?;
            println!("Saved {}", project.display());
        }
        Action::Probe { input } => {
            let p = backend.open(&input)?;
            let mut value = serde_json::to_value(&p)?;
            value["audio_streams"] = serde_json::to_value(backend.audio_streams(&input)?)?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        Action::Import {
            project,
            password_stdin,
            inputs,
        } => {
            let password = read_password(password_stdin)?;
            let mut p = Project::load(&project)?;
            p.import_assets_with(
                &backend,
                inputs,
                &project.with_extension("assets"),
                &backend.job,
                password.as_deref(),
            )?;
            p.save(&project)?;
        }
        Action::Encoders => {
            for e in backend.encoders()? {
                println!("{e}");
            }
        }
        Action::Check { project } => {
            let p = Project::load(&project)?;
            p.validate()?;
            for t in &p.tracks {
                let fresh = backend.probe_stream(&t.path, t.stream_index)?;
                anyhow::ensure!(
                    fresh.track.duration_ms.abs_diff(t.duration_ms) <= 100,
                    "Source changed"
                );
            }
            println!(
                "Valid: {} tracks, {} ms, estimated {} bytes AAC",
                p.tracks.len(),
                p.duration_ms(),
                p.estimated_bytes()
            );
        }
        Action::Copy {
            project,
            destination,
        } => {
            println!(
                "{}",
                Project::load(&project)?
                    .copy_to_with_job(&destination, &backend.job)?
                    .display()
            );
        }
        Action::Stream {
            project,
            track,
            index,
        } => {
            let mut p = Project::load(&project)?;
            p.select_stream(&backend, track, index)?;
            p.save(&project)?;
        }
        Action::Preview {
            project,
            output,
            start_ms,
            duration_ms,
            format,
        } => {
            let mut p = Project::load(&project)?;
            if let Some(format) = format {
                p.settings.format = parse_format(&format);
            }
            backend.encoded_preview(&p, &output, start_ms, duration_ms, cancel, &mut |e| {
                eprintln!("{e:?}")
            })?;
        }
        Action::Export {
            project,
            output,
            format,
        } => {
            let mut p = Project::load(&project)?;
            if let Some(format) = format {
                p.settings.format = parse_format(&format);
            }
            backend.export(&p, &output, cancel, &mut |e| match e {
                ExportEvent::Log(s) => eprintln!("{s}"),
                ExportEvent::Progress(p) => eprintln!("{:.0}%", p * 100.),
                ExportEvent::Phase(_) | ExportEvent::Activity { .. } => {}
            })?;
        }
        Action::Mcp => mcp(&backend)?,
    }
    Ok(())
}
fn mcp(backend: &DesktopFfmpeg) -> Result<()> {
    use serde_json::{json, Value};
    // Intentionally read-only MCP: project writes and exports stay explicit CLI/UI operations.
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.len() > 1_048_576 {
            bail!("MCP request too large");
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                println!(
                    "{}",
                    json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}})
                );
                continue;
            }
        };
        if req["jsonrpc"] != "2.0"
            || !req["method"].is_string()
            || req
                .get("id")
                .is_some_and(|id| !id.is_null() && !id.is_string() && !id.is_number())
        {
            println!(
                "{}",
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid Request"}})
            );
            continue;
        }
        let Some(id) = req.get("id") else {
            continue;
        };
        let result: Result<Value> = (|| {
            Ok(match req["method"].as_str().unwrap_or("") {
                "initialize" => {
                    json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"M4B Editor","version":env!("CARGO_PKG_VERSION")}})
                }
                "ping" => json!({}),
                "tools/list" => {
                    json!({"tools":[{"name":"probe","description":"Read local audio metadata and chapters","inputSchema":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}},{"name":"inspect_project","description":"Read a local M4B Editor project","inputSchema":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}}]})
                }
                "tools/call" => {
                    let path = PathBuf::from(
                        req["params"]["arguments"]["path"]
                            .as_str()
                            .context("Missing path")?,
                    );
                    let data = match req["params"]["name"].as_str().unwrap_or("") {
                        "probe" => backend.open(&path),
                        "inspect_project" => Project::load(&path),
                        _ => bail!("Unknown tool"),
                    };
                    match data {
                        Ok(p) => {
                            json!({"content":[{"type":"text","text":serde_json::to_string_pretty(&p)?}]})
                        }
                        Err(e) => {
                            json!({"isError":true,"content":[{"type":"text","text":e.to_string()}]})
                        }
                    }
                }
                _ => bail!("Method not found"),
            })
        })();
        let response = match result {
            Ok(v) => json!({"jsonrpc":"2.0","id":id,"result":v}),
            Err(e) => {
                let code = if e.to_string() == "Method not found" {
                    -32601
                } else {
                    -32602
                };
                json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":e.to_string()}})
            }
        };
        println!("{response}");
        io::stdout().flush()?;
    }
    Ok(())
}

fn parse_format(value: &str) -> m4b_editor_core::ExportFormat {
    match value {
        "mp3" => m4b_editor_core::ExportFormat::Mp3,
        "flac" => m4b_editor_core::ExportFormat::Flac,
        _ => m4b_editor_core::ExportFormat::M4b,
    }
}

fn read_password(requested: bool) -> Result<Option<String>> {
    if !requested {
        return Ok(None);
    }
    use std::io::IsTerminal;
    anyhow::ensure!(
        !io::stdin().is_terminal(),
        "Use a pipe for --password-stdin to avoid terminal echo"
    );
    let mut password = String::new();
    io::stdin().lock().take(4097).read_line(&mut password)?;
    anyhow::ensure!(password.len() <= 4096, "Password exceeds limit");
    Ok(Some(password.trim_end_matches(['\r', '\n']).to_owned()))
}
