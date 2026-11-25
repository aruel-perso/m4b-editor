#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod catalog;
mod diagnostics;
mod export_timing;
#[cfg(any(windows, test))]
mod instance;
mod playback;
mod preferences;
mod recovery;
mod ui_media;
mod updater;
use m4b_editor_core::{
    Cancel, Chapter, DesktopFfmpeg, ExportEvent, ExportFormat, ExportPhase, ImportEvent,
    MediaBackend, Project, Settings,
};
use playback::{CommandMessage as AudioCommand, Playback};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
const APP_TITLE: &str = concat!("M4B Editor ", env!("CARGO_PKG_VERSION"));

fn desktop_viewport() -> egui::ViewportBuilder {
    let icon =
        eframe::icon_data::from_png_bytes(include_bytes!("../../../packaging/icons/app.png"))
            .expect("The embedded application icon must be a valid PNG");
    egui::ViewportBuilder::default()
        .with_inner_size([1000., 800.])
        .with_title(APP_TITLE)
        .with_icon(icon)
}

#[test]
fn window_uses_embedded_app_icon() {
    let icon = desktop_viewport().icon.unwrap();
    assert_eq!((icon.width, icon.height), (64, 64));
    assert_eq!(icon.rgba.len(), 64 * 64 * 4);
    assert!(icon
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[3] == 0));
    assert!(icon
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel == &[255, 191, 85, 255]));
}

fn main() -> eframe::Result<()> {
    #[cfg(windows)]
    let primary = match instance::start() {
        Ok(Some(primary)) => primary,
        Ok(None) => return Ok(()),
        Err(error) => {
            rfd::MessageDialog::new()
                .set_title(APP_TITLE)
                .set_description(format!("Cannot start or activate M4B Editor: {error}"))
                .show();
            return Ok(());
        }
    };
    diagnostics::init();
    eframe::run_native(
        "M4B Editor",
        eframe::NativeOptions {
            viewport: desktop_viewport(),
            ..Default::default()
        },
        Box::new(move |_| {
            Ok(Box::new(App {
                #[cfg(windows)]
                instance: Some(primary),
                ..App::default()
            }))
        }),
    )
}
use eframe::egui;
enum Message {
    Copied(Result<PathBuf, String>),
    Streams(usize, Result<Vec<m4b_editor_core::AudioStream>, String>),
    Preview(tempfile::TempDir, Result<PathBuf, String>),
    Cover(PathBuf, Result<egui::ColorImage, String>),
    RecentCover(PathBuf, Option<egui::ColorImage>),
    UpdateProgress(f32),
    Encoders(Result<bool, String>),
    UpdateChecked(updater::Channel, Result<updater::Verified, String>),
    UpdateDownloaded(Result<updater::Download, String>),
    UpdateInstalled(Result<(), String>),
    Loaded(Box<Result<Project, String>>, Option<Option<PathBuf>>),
    ImportedM4b(Box<Result<Project, String>>),
    LoadedProject(
        PathBuf,
        Box<Result<(Project, m4b_editor_core::ProjectBaseline), String>>,
    ),
    Event(ExportEvent, Instant),
    ImportProgress(ImportEvent),
    Done(Result<Settings, String>, Instant),
}
#[derive(Clone, Copy, PartialEq)]
enum Section {
    Project,
    Metadata,
    Chapters,
}
struct App {
    pending_recoveries: Vec<PathBuf>,
    settings_open: bool,
    recent_covers: HashMap<PathBuf, Option<egui::TextureHandle>>,
    recent_loading: Option<PathBuf>,
    remove_recent: Option<PathBuf>,
    export_settings: Option<Settings>,
    window_title: String,
    restored_from: Option<PathBuf>,
    recovery_store: recovery::Recovery,
    encoded_preview: Option<tempfile::TempDir>,
    archive_password: String,
    selected_stream: u32,
    stream_track: Option<usize>,
    stream_choices: Vec<m4b_editor_core::AudioStream>,
    prefs: preferences::Preferences,
    selected: bool,
    startup_checked: bool,
    checking: bool,
    cover_path: Option<PathBuf>,
    cover_texture: Option<egui::TextureHandle>,
    cover_error: String,
    update_channel: updater::Channel,
    update_candidate: Option<updater::Verified>,
    section: Section,
    creating: bool,
    new_name: String,
    allow_close: bool,
    project: Project,
    path: Option<PathBuf>,
    baseline: Option<m4b_editor_core::ProjectBaseline>,
    backend: DesktopFfmpeg,
    tx: mpsc::Sender<Message>,
    rx: mpsc::Receiver<Message>,
    busy: bool,
    exporting: bool,
    export_phase: Option<ExportPhase>,
    export_result: Option<Result<(), String>>,
    export_timing: Option<export_timing::Timing>,
    import_progress: Option<ImportEvent>,
    #[cfg(windows)]
    instance: Option<instance::Primary>,
    cancel: Cancel,
    progress: f32,
    logs: Vec<String>,
    status: String,
    last_save: Instant,
    dirty: bool,
    recovery: Option<recovery::Prompt>,
    bitrate_available: Option<bool>,
    playback: Playback,
    preview: playback::State,
    preview_track: Option<usize>,
    volume: f32,
    seek_ms: u64,
}
impl Default for App {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        let recovery_store = recovery::Recovery::new(&state_directory().join("recovery"))
            .expect("Create recovery directory");
        let pending_recoveries = recovery::Recovery::discover(&state_directory().join("recovery"));
        let installer_error = std::fs::File::open(updater::error_log())
            .ok()
            .and_then(|file| {
                use std::io::Read;
                let mut text = String::new();
                file.take(16384).read_to_string(&mut text).ok()?;
                Some(text)
            })
            .filter(|s| !s.trim().is_empty());
        let legacy_preferences = if cfg!(test) {
            None
        } else {
            legacy_state_directory().map(|p| p.join("preferences.json"))
        };
        let prefs = preferences::load_migrated(&preferences_path(), legacy_preferences.as_deref());
        let mut project = Project::default();
        project.settings.language = prefs.language.clone();
        let channel = if prefs.prerelease {
            updater::Channel::Prerelease
        } else {
            updater::Channel::Stable
        };
        Self {
            pending_recoveries,
            settings_open: false,
            recent_covers: HashMap::new(),
            recent_loading: None,
            remove_recent: None,
            export_settings: None,
            window_title: APP_TITLE.into(),
            restored_from: None,
            recovery_store,
            encoded_preview: None,
            archive_password: String::new(),
            selected_stream: 0,
            stream_track: None,
            stream_choices: Vec::new(),
            prefs,
            selected: false,
            startup_checked: false,
            checking: false,
            cover_path: None,
            cover_texture: None,
            cover_error: String::new(),
            update_channel: channel,
            update_candidate: None,
            section: Section::Project,
            creating: false,
            new_name: String::new(),
            allow_close: false,
            project,
            path: None,
            baseline: None,
            backend: DesktopFfmpeg::default(),
            tx,
            rx,
            busy: false,
            exporting: false,
            export_phase: None,
            export_result: None,
            export_timing: None,
            import_progress: None,
            #[cfg(windows)]
            instance: None,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: 0.,
            logs: installer_error.clone().into_iter().collect(),
            status: installer_error.unwrap_or_default(),
            last_save: Instant::now(),
            dirty: false,
            recovery: None,
            bitrate_available: None,
            playback: Playback::new(),
            preview: playback::State::default(),
            preview_track: None,
            volume: 1.,
            seek_ms: 0,
        }
    }
}
fn configured_state_root(
    platform: &str,
    env: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    let absolute = |key| {
        env(key)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    if platform == "windows" {
        absolute("LOCALAPPDATA")
    } else {
        absolute("XDG_STATE_HOME")
            .or_else(|| absolute("HOME").map(|home| home.join(".local/state")))
    }
}

fn legacy_state_directory() -> Option<PathBuf> {
    let root = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_STATE_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(root.join("m4b-Craft")) // 0.3 compatibility: preserve originals.
}

#[cfg(test)]
fn state_directory() -> PathBuf {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| tempfile::tempdir().unwrap().keep())
        .clone()
}

#[cfg(unix)]
fn private_unix_state(temp: &Path) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let uid = unsafe { libc::geteuid() };
    let path = temp.join(format!("m4b-editor-{uid}"));
    let mut builder = std::fs::DirBuilder::new();
    match builder.mode(0o700).create(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let info = path.symlink_metadata()?;
    if !info.is_dir() || info.uid() != uid || info.mode() & 0o7777 != 0o700 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Temporary application state directory is not private",
        ));
    }
    Ok(path)
}

#[cfg(not(test))]
fn state_directory() -> PathBuf {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        if let Some(root) = configured_state_root(std::env::consts::OS, |key| std::env::var_os(key))
        {
            return root.join("m4b-editor");
        }
        #[cfg(unix)]
        if let Ok(path) = private_unix_state(&std::env::temp_dir()) {
            return path;
        }
        tempfile::Builder::new()
            .prefix("m4b-editor-")
            .tempdir()
            .expect("Create private temporary application state directory")
            .keep()
    })
    .clone()
}
fn preferences_path() -> PathBuf {
    state_directory().join("preferences.json")
}

impl App {
    fn check_updates(&mut self) {
        self.checking = true;
        let tx = self.tx.clone();
        let channel = self.update_channel;
        std::thread::spawn(move || {
            let _ = tx.send(Message::UpdateChecked(
                channel,
                updater::check(channel, false).map_err(|e| format!("Update check: {e:#}")),
            ));
        });
    }
    fn show_settings(&mut self, ctx: &egui::Context) {
        let fr = self.prefs.language == "fr";
        let label = |key| catalog::text(fr, key);
        let mut changed = false;
        modal(ctx, "application_settings", |ui| {
            ui.heading(label("Settings"));
            ui.label(label("Language"));
            ui.horizontal(|ui| {
                changed |= ui
                    .selectable_value(&mut self.prefs.language, "en".into(), "English")
                    .changed();
                changed |= ui
                    .selectable_value(&mut self.prefs.language, "fr".into(), "Français")
                    .changed();
            });
            ui.label(label("Theme"));
            ui.horizontal_wrapped(|ui| {
                for (value, key) in [
                    (None, "Use project theme"),
                    (Some(preferences::Theme::System), "System"),
                    (Some(preferences::Theme::Light), "Light"),
                    (Some(preferences::Theme::Dark), "Dark"),
                ] {
                    changed |= ui
                        .selectable_value(&mut self.prefs.theme, value, label(key))
                        .changed();
                }
            });
            if ui
                .add(
                    egui::Slider::new(&mut self.prefs.sidebar, 120.0..=400.0)
                        .integer()
                        .text(label("Sidebar width")),
                )
                .changed()
            {
                ctx.data_mut(|data| {
                    data.remove::<egui::panel::PanelState>(egui::Id::new("navigation"))
                });
                changed = true;
            }
            ui.separator();
            ui.label(label(
                "GitHub updates are checked automatically at startup.",
            ));
            let previous = self.update_channel;
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut self.update_channel,
                    updater::Channel::Stable,
                    label("Stable"),
                );
                ui.selectable_value(
                    &mut self.update_channel,
                    updater::Channel::Prerelease,
                    label("Include prereleases"),
                );
            });
            if previous != self.update_channel {
                self.update_candidate = None;
                self.prefs.prerelease = self.update_channel == updater::Channel::Prerelease;
                changed = true;
            }
            if ui.button(label("Close")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape))
            {
                self.settings_open = false;
            }
        });
        if changed {
            let _ = self.prefs.save(&preferences_path());
        }
    }

    fn show_recovery(&mut self, ctx: &egui::Context) {
        let prompt = self.recovery.as_ref().unwrap();
        let mut restore = false;
        let mut dismiss = false;
        modal(ctx, "recover_project", |ui| {
            ui.heading(self.text("Recover unsaved project"));
            match &prompt.name {
                Ok(name) => {
                    ui.label(self.text(
                        "An autosave contains unsaved project changes that you can restore.",
                    ));
                    if !name.is_empty() {
                        ui.label(format!("{} · {name}", self.text("Project name")));
                    }
                }
                Err(_) => {
                    ui.label(self.text("This autosave could not be read."));
                }
            }
            if let Some(date) = &prompt.date {
                ui.label(format!("{} · {date}", self.text("Autosave date")));
            }
            ui.horizontal(|ui| {
                restore = ui
                    .add_enabled(prompt.name.is_ok(), egui::Button::new(self.text("Restore")))
                    .clicked();
                dismiss = ui.button(self.text("Dismiss")).clicked();
            });
        });
        if restore {
            self.restore_recovery();
        } else if dismiss {
            self.dismiss_recovery();
        }
    }

    fn dismiss_recovery(&mut self) {
        if let Some(prompt) = self.recovery.take() {
            if let Err(error) = recovery::resolve(&prompt.path) {
                self.log_error(error.to_string());
            }
        }
    }

    fn clear_recovery(&mut self) {
        self.recovery_store.clear();
        if let Some(path) = self.restored_from.take() {
            if let Err(error) = recovery::resolve(&path) {
                self.log_error(error.to_string());
            }
        }
    }

    fn restore_recovery(&mut self) {
        if self.busy || self.creating {
            return;
        }
        let Some(prompt) = self.recovery.as_ref() else {
            return;
        };
        let path = prompt.path.clone();
        let project = match Project::load(&path) {
            Ok(project) => project,
            Err(error) => {
                self.log_error(error.to_string());
                self.recovery = Some(recovery::Prompt::new(path));
                return;
            }
        };
        if !self.confirm_replace() {
            return;
        }
        self.recovery = None;
        self.clear_recovery();
        self.stop_preview();
        self.project = project;
        self.stream_track = None;
        self.stream_choices.clear();
        self.selected = true;
        self.path = None;
        self.baseline = None;
        self.dirty = true;
        self.last_save = Instant::now();
        self.restored_from = Some(path);
    }

    fn close_project(&mut self) {
        if self.selected && !self.busy && !self.creating && self.confirm_replace() {
            self.stop_preview();
            self.clear_recovery();
            self.selected = false;
            self.project = Project::default();
            self.path = None;
            self.baseline = None;
            self.restored_from = None;
            self.dirty = false;
        }
    }

    fn finish_frame(&mut self, ctx: &egui::Context, recovery_open: bool) {
        if self.exporting || self.export_result.is_some() {
            self.show_running_export(ctx);
        } else if recovery_open {
            self.show_recovery(ctx);
        } else if self.settings_open {
            self.show_settings(ctx);
        } else if self.remove_recent.is_some() {
            self.show_remove_recent(ctx);
        } else if self.export_settings.is_some() {
            self.show_export(ctx);
        }
        let title = if self.selected {
            format!("{APP_TITLE} - {}", self.project.name)
        } else {
            APP_TITLE.into()
        };
        if self.window_title != title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn remember(&mut self) {
        if let Some(path) = self.path.clone() {
            self.prefs.remember(path);
            self.recent_covers
                .retain(|path, _| self.prefs.recent.contains(path));
        }
        let _ = self.prefs.save(&preferences_path());
    }

    fn queue_recent_cover(&mut self) {
        if self.recent_loading.is_some() {
            return;
        }
        let Some(path) = self
            .prefs
            .recent
            .iter()
            .find(|path| !self.recent_covers.contains_key(*path))
            .cloned()
        else {
            return;
        };
        self.recent_loading = Some(path.clone());
        let mut backend = self.backend.clone();
        backend.job =
            m4b_editor_core::Job::new(Arc::new(AtomicBool::new(false)), Duration::from_secs(5));
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let image = ui_media::recent_cover(&backend, &path);
            let _ = tx.send(Message::RecentCover(path, image));
        });
    }

    fn finish_remove_recent(&mut self, confirmed: bool, prefs_path: &Path) {
        if let Some(path) = self.remove_recent.take().filter(|_| confirmed) {
            let previous = self.prefs.recent.clone();
            self.prefs.remove(&path);
            if let Err(error) = self.prefs.save(prefs_path) {
                self.prefs.recent = previous;
                self.log_error(error.to_string());
            } else {
                self.recent_covers.remove(&path);
            }
        }
    }

    fn show_remove_recent(&mut self, ctx: &egui::Context) {
        let mut choice = ctx
            .input(|i| i.key_pressed(egui::Key::Escape))
            .then_some(false);
        modal(ctx, "remove-recent", |ui| {
            ui.heading(self.text("Remove from recent projects?"));
            ui.label(self.remove_recent.as_ref().unwrap().display().to_string());
            ui.label(self.text("The project and its files will be kept."));
            ui.horizontal(|ui| {
                if ui.button(self.text("Remove from recents")).clicked() {
                    choice = Some(true);
                }
                if ui.button(self.text("Cancel")).clicked() {
                    choice = Some(false);
                }
            });
        });
        if let Some(confirmed) = choice {
            self.finish_remove_recent(confirmed, &preferences_path());
        }
    }

    fn load_path(&mut self, path: PathBuf) {
        let is_project = path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("json") || s.eq_ignore_ascii_case("m4bproj"));
        let project_path = if is_project { Some(path.clone()) } else { None };
        self.cancel = Arc::new(AtomicBool::new(false));
        let mut b = self.backend.clone();
        b.job = m4b_editor_core::Job::new(self.cancel.clone(), Duration::from_secs(120));
        let tx = self.tx.clone();
        self.busy = true;
        std::thread::spawn(move || {
            if is_project {
                let result = Project::load_with_baseline(&path).map_err(|e| format!("{e:#}"));
                let _ = tx.send(Message::LoadedProject(path, Box::new(result)));
            } else {
                let result = b.open(&path).map_err(|e| format!("{e:#}"));
                let _ = tx.send(Message::Loaded(Box::new(result), Some(project_path)));
            }
        });
    }
    fn confirm_replace(&mut self) -> bool {
        if !self.dirty {
            return true;
        }
        let choice = rfd::MessageDialog::new()
            .set_title(self.text("Unsaved project"))
            .set_description(self.text("Save changes before continuing?"))
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .show();
        if choice == rfd::MessageDialogResult::Yes {
            self.save(false);
        }
        replacement_allowed(choice, !self.dirty)
    }

    fn navigate_preview(&mut self, next: bool) {
        let Some(index) = self.preview_track else {
            return;
        };
        if self.project.tracks.len() == 1 && !self.project.chapters.is_empty() {
            let position = self.preview.position_ms;
            let target = if next {
                self.project
                    .chapters
                    .iter()
                    .find(|c| c.start_ms > position)
                    .map(|c| c.start_ms)
            } else {
                self.project
                    .chapters
                    .iter()
                    .rev()
                    .find(|c| c.start_ms < position.saturating_sub(1000))
                    .map(|c| c.start_ms)
                    .or(Some(0))
            };
            if let Some(ms) = target {
                self.play_preview(0, ms);
            }
        } else {
            let target = if next {
                index
                    .checked_add(1)
                    .filter(|i| *i < self.project.tracks.len())
            } else {
                index.checked_sub(1)
            };
            if let Some(i) = target {
                self.play_preview(i, 0);
            }
        }
    }
    fn stop_preview(&mut self) {
        self.playback.send(AudioCommand::Stop);
        self.preview_track = None;
        self.preview = playback::State::default();
    }
    fn play_preview(&mut self, index: usize, position: u64) {
        if let Some(track) = self.project.tracks.get(index) {
            self.playback.send(AudioCommand::Play(
                track.path.clone(),
                position.min(track.duration_ms),
                track.stream_index,
            ));
            self.preview_track = Some(index);
            self.seek_ms = position.min(track.duration_ms);
        }
    }
    fn text(&self, key: &str) -> &'static str {
        catalog::text(self.prefs.language == "fr", key)
    }
    fn log(&mut self, s: String) {
        let translated = catalog::message(self.prefs.language == "fr", &s);
        self.status =
            if translated != s || catalog::CATALOG.iter().any(|(en, fr)| *en == s || *fr == s) {
                translated.clone()
            } else {
                self.text("Operation details").into()
            };
        self.logs.push(translated);
        if self.logs.len() > 500 {
            self.logs.remove(0);
        }
    }
    fn log_error(&mut self, diagnostic: String) {
        diagnostics::operation_error();
        self.log(diagnostic.clone());
        self.status = catalog::error(self.prefs.language == "fr", &diagnostic).into();
    }
    fn save(&mut self, choose: bool) {
        if !self.selected {
            return;
        }
        let path = if choose || self.path.is_none() {
            rfd::FileDialog::new()
                .add_filter(self.text("M4B Editor project"), &["m4bproj"])
                .set_file_name("book.m4bproj")
                .save_file()
                .and_then(|p| {
                    let target = project_save_target(&p);
                    let exists = target.exists();
                    let consent = !exists
                        || rfd::MessageDialog::new()
                            .set_title(self.text("Overwrite project"))
                            .set_description(format!(
                                "{} {} ?",
                                self.text("Replace"),
                                target.display()
                            ))
                            .set_buttons(rfd::MessageButtons::YesNo)
                            .show()
                            == rfd::MessageDialogResult::Yes;
                    overwrite_allowed(exists, consent).then_some(target)
                })
        } else {
            self.path.clone()
        };
        if let Some(path) = path {
            let expected = if choose || self.path.is_none() {
                None
            } else {
                self.baseline.as_ref()
            };
            match self.project.save_with_baseline(&path, expected) {
                Ok(baseline) => {
                    self.baseline = Some(baseline);
                    self.clear_recovery();
                    self.recent_covers.remove(&path);
                    self.path = Some(path);
                    self.remember();
                    self.dirty = false;
                    diagnostics::project_saved();
                    self.log(self.text("Project saved").into());
                }
                Err(e) => self.log_error(format!("{}: {e:#}", self.text("Save error"))),
            }
        }
    }
    fn import_m4b(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("M4B", &["m4b"])
            .pick_file()
        else {
            return;
        };
        self.cancel = Arc::new(AtomicBool::new(false));
        let mut backend = self.backend.clone();
        backend.job = m4b_editor_core::Job::new(self.cancel.clone(), Duration::from_secs(120));
        let tx = self.tx.clone();
        self.busy = true;
        std::thread::spawn(move || {
            let result = ui_media::open_m4b(&backend, &path).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Message::ImportedM4b(Box::new(result)));
        });
    }

    fn import(&mut self) {
        if !self.selected {
            return;
        }
        if self.path.is_none() {
            if !self.confirm_replace() {
                return;
            }
            self.creating = true;
            self.new_name = self.project.metadata.title.clone();
            self.log(
                self.text("Create a named project first, then import")
                    .into(),
            );
            return;
        }
        let Some(project_path) = self.path.clone() else {
            return;
        };
        let assets = project_path.with_extension("assets");
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter(
                self.text("Audio"),
                &[
                    "mka", "mkv", "mp4", "m4b", "m4a", "mp3", "flac", "ogg", "opus", "wav", "aac",
                    "zip", "rar",
                ],
            )
            .pick_files()
        {
            let mut p = self.project.clone();
            self.cancel = Arc::new(AtomicBool::new(false));
            let mut b = self.backend.clone();
            b.job = m4b_editor_core::Job::new(self.cancel.clone(), Duration::from_secs(600));
            let password = std::mem::take(&mut self.archive_password);
            let tx = self.tx.clone();
            self.busy = true;
            self.import_progress = Some(ImportEvent::Inspecting);
            let mut diagnostic = diagnostics::Job::import(paths.len(), !password.is_empty());
            std::thread::spawn(move || {
                let result = p.import_assets_observed(
                    &b,
                    paths,
                    &assets,
                    &b.job,
                    (!password.is_empty()).then_some(password.as_str()),
                    &mut |event| {
                        diagnostic.import_event(event);
                        let _ = tx.send(Message::ImportProgress(event));
                    },
                );
                diagnostic.finish(&result, b.job.cancel.load(Ordering::Relaxed));
                let _ = tx.send(Message::Loaded(
                    Box::new(result.map(|()| p).map_err(|e| format!("{e:#}"))),
                    None,
                ));
            });
        }
    }
    fn open(&mut self) {
        if !self.confirm_replace() {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(
                self.text("Project"),
                &[
                    "m4bproj", "json", "m4b", "m4a", "mp3", "flac", "mka", "wav", "ogg", "opus",
                ],
            )
            .pick_file()
        {
            self.load_path(path);
        }
    }
    fn encode_preview(&mut self, settings: Settings, transition: bool) {
        let track_index = self
            .preview_track
            .unwrap_or(0)
            .min(self.project.tracks.len().saturating_sub(1));
        let book_position = self
            .project
            .tracks
            .iter()
            .take(track_index)
            .map(|t| t.duration_ms)
            .sum::<u64>()
            .saturating_add(self.seek_ms);
        let boundary = if self.project.tracks.len() > 1 {
            self.project
                .tracks
                .iter()
                .take(track_index.max(1))
                .map(|t| t.duration_ms)
                .sum::<u64>()
        } else {
            self.project
                .chapters
                .iter()
                .find(|c| c.start_ms > book_position)
                .map(|c| c.start_ms)
                .unwrap_or(book_position)
        };
        let start = if transition {
            boundary.saturating_sub(3000)
        } else {
            book_position.min(self.project.duration_ms().saturating_sub(1))
        };
        self.stop_preview();
        let directory = match tempfile::tempdir() {
            Ok(d) => d,
            Err(e) => {
                self.log(e.to_string());
                return;
            }
        };
        let output = directory
            .path()
            .join(format!("preview.{}", settings.format.extension()));
        let mut p = self.project.clone();
        p.settings = settings;
        let mut b = self.backend.clone();
        let tx = self.tx.clone();
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        b.job = m4b_editor_core::Job::new(cancel.clone(), Duration::from_secs(600));
        self.busy = true;
        self.exporting = true;
        std::thread::spawn(move || {
            let result = b
                .encoded_preview(
                    &p,
                    &output,
                    start,
                    if transition { 6000 } else { 10000 },
                    cancel,
                    &mut |e| {
                        let _ = tx.send(Message::Event(e, Instant::now()));
                    },
                )
                .map(|_| output)
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Message::Preview(directory, result));
        });
    }
    fn show_export_settings(&mut self) {
        self.export_settings = Some(self.project.settings.clone());
        self.bitrate_available = None;
    }

    fn show_export(&mut self, ctx: &egui::Context) {
        let mut draft = self.export_settings.take().unwrap();
        let mut confirm = false;
        let mut dismiss = !self.busy && ctx.input(|i| i.key_pressed(egui::Key::Escape));
        modal(ctx, "audio-export", |ui| {
            ui.heading(self.text("Audio export"));
            ui.add_enabled_ui(!self.busy, |ui| {
                ui.horizontal(|ui| {
                    for (format, label) in [
                        (ExportFormat::M4b, "M4B (AAC)"),
                        (ExportFormat::Mp3, "MP3"),
                        (ExportFormat::Flac, "FLAC"),
                    ] {
                        if ui
                            .selectable_value(&mut draft.format, format, label)
                            .changed()
                        {
                            self.bitrate_available = None;
                        }
                    }
                });
                if draft.format == ExportFormat::M4b {
                    ui.checkbox(
                        &mut draft.lossless_remux,
                        self.text("Lossless remux for single AAC M4B/M4A"),
                    );
                }
                let remux = draft.format == ExportFormat::M4b
                    && draft.lossless_remux
                    && matches!(self.project.tracks.as_slice(), [track]
                    if track.codec == "aac" && track.path.extension().is_some_and(|ext|
                        ext.eq_ignore_ascii_case("m4b") || ext.eq_ignore_ascii_case("m4a")
                    ));
                let rates = ui_media::bitrates(draft.format);
                if draft.format == ExportFormat::Flac {
                    ui.label(self.text("FLAC uses lossless compression; bitrate is automatic."));
                } else if remux {
                    ui.label(
                        self.text("Audio is copied without re-encoding; bitrate is preserved."),
                    );
                } else {
                    if !rates.contains(&draft.bitrate_kbps) {
                        draft.bitrate_kbps = Settings::default().bitrate_kbps;
                    }
                    ui.horizontal(|ui| {
                        ui.label(self.text("Bitrate (kb/s)"));
                        egui::ComboBox::from_id_salt("bitrate")
                            .selected_text(draft.bitrate_kbps.to_string())
                            .show_ui(ui, |ui| {
                                for &rate in rates {
                                    ui.selectable_value(
                                        &mut draft.bitrate_kbps,
                                        rate,
                                        rate.to_string(),
                                    );
                                }
                            });
                    });
                }
                if ui.button(self.text("Check encoder")).clicked() {
                    self.busy = true;
                    self.cancel = Arc::new(AtomicBool::new(false));
                    let mut backend = self.backend.clone();
                    backend.job =
                        m4b_editor_core::Job::new(self.cancel.clone(), Duration::from_secs(20));
                    let encoder = draft.format.encoder();
                    let tx = self.tx.clone();
                    std::thread::spawn(move || {
                        let _ = tx.send(Message::Encoders(
                            backend
                                .encoders()
                                .map(|e| e.iter().any(|e| e == encoder))
                                .map_err(|e| format!("{e:#}")),
                        ));
                    });
                }
                if let Some(available) = self.bitrate_available {
                    ui.label(if available {
                        self.text("Encoder available")
                    } else {
                        self.text("Encoder unavailable")
                    });
                }
                ui.horizontal(|ui| {
                    if ui.button(self.text("Encoded excerpt (10 s)")).clicked() {
                        self.encode_preview(draft.clone(), false);
                    }
                    if ui.button(self.text("Encoded transition (6 s)")).clicked() {
                        self.encode_preview(draft.clone(), true);
                    }
                    if self.encoded_preview.is_some() && ui.button(self.text("Stop")).clicked() {
                        self.stop_preview();
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    confirm = ui.button(self.text("Continue to save…")).clicked();
                    dismiss |= ui.button(self.text("Cancel")).clicked();
                });
            });
            if self.busy {
                ui.add(egui::ProgressBar::new(self.progress).show_percentage());
                if ui.button(self.text("Cancel operation")).clicked() {
                    self.cancel.store(true, Ordering::Relaxed);
                }
            }
        });
        if dismiss {
            if self.encoded_preview.is_some() {
                self.stop_preview();
            }
            return;
        }
        if confirm && self.export(&draft) {
            return;
        }
        self.export_settings = Some(draft);
    }

    fn show_running_export(&mut self, ctx: &egui::Context) {
        let mut close = false;
        modal(ctx, "running-export", |ui| {
            ui.heading(self.text("Audio export"));
            if let Some(timing) = &self.export_timing {
                let now = Instant::now();
                let elapsed = export_timing::display(timing.elapsed(now));
                if self.exporting {
                    let total = timing.estimate(now).map_or_else(
                        || self.text("Unknown").into(),
                        |estimate| format!("~{}", export_timing::display(estimate)),
                    );
                    ui.label(format!(
                        "{} {elapsed} / {total}",
                        self.text("Elapsed / estimated total")
                    ));
                } else {
                    ui.label(format!("{} {elapsed}", self.text("Elapsed")));
                }
            }
            if let Some(result) = &self.export_result {
                match result {
                    Ok(()) => {
                        ui.label(self.text("Export complete"));
                    }
                    Err(error) => {
                        ui.label(catalog::error(self.prefs.language == "fr", error));
                        egui::ScrollArea::vertical()
                            .max_height(140.)
                            .show(ui, |ui| {
                                ui.label(error);
                            });
                    }
                }
                close = ui.button(self.text("Close")).clicked()
                    || ctx.input(|i| i.key_pressed(egui::Key::Escape));
            } else {
                let phase = match self.export_phase {
                    None => "Preparing export…",
                    Some(ExportPhase::CheckingSources) => "Checking source files…",
                    Some(ExportPhase::PreparingTrack(_)) => "Preparing audio tracks…",
                    Some(ExportPhase::Encoding) => "Encoding audio…",
                    Some(ExportPhase::CheckingOutput) => "Checking output…",
                    Some(ExportPhase::VerifyingAudio) => "Verifying audio…",
                    Some(ExportPhase::Publishing) => "Saving output…",
                };
                if self.cancel.load(Ordering::Relaxed) {
                    ui.label(self.text("Cancellation requested…"));
                } else {
                    ui.label(self.text(phase));
                }
                if self.progress > 0. {
                    ui.add(egui::ProgressBar::new(self.progress).show_percentage());
                } else {
                    ui.spinner();
                }
                if ui
                    .add_enabled(
                        !self.cancel.load(Ordering::Relaxed),
                        egui::Button::new(self.text("Cancel operation")),
                    )
                    .clicked()
                {
                    self.cancel.store(true, Ordering::Relaxed);
                }
            }
        });
        if close {
            self.export_result = None;
            self.export_timing = None;
        }
    }

    fn export(&mut self, draft: &Settings) -> bool {
        let mut settings = draft.clone();
        // The core schema validates the stored bitrate even when the encoder ignores it.
        if !ui_media::bitrates(ExportFormat::M4b).contains(&settings.bitrate_kbps) {
            settings.bitrate_kbps = Settings::default().bitrate_kbps;
        }
        let mut project = self.project.clone();
        project.settings = settings.clone();
        if let Err(e) = project.validate() {
            self.log_error(format!("{}: {e:#}", self.text("Validation")));
            return false;
        }
        let extension = settings.format.extension();
        let Some(output) = rfd::FileDialog::new()
            .add_filter(self.text("Audio"), &[extension])
            .set_file_name(format!("audiobook.{extension}"))
            .save_file()
        else {
            return false;
        };
        let output = if output.extension().is_none() {
            output.with_extension(extension)
        } else {
            output
        };
        if !output
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
        {
            self.log(
                self.text("The filename extension must match the export format.")
                    .into(),
            );
            return false;
        }
        let replacement = match output.symlink_metadata() {
            Ok(_) => {
                let protected = self
                    .path
                    .iter()
                    .chain(project.cover.iter())
                    .chain(project.tracks.iter().map(|track| &track.path));
                let replacement = match ui_media::Replacement::prepare(&output, protected) {
                    Ok(replacement) => replacement,
                    Err(error) => {
                        self.log_error(format!("{error:#}"));
                        return false;
                    }
                };
                let consent = rfd::MessageDialog::new()
                    .set_title(self.text("Replace exported audio?"))
                    .set_description(output.display().to_string())
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show();
                if consent != rfd::MessageDialogResult::Yes {
                    return false;
                }
                Some(replacement)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                self.log_error(error.to_string());
                return false;
            }
        };
        self.stop_preview();
        self.busy = true;
        self.exporting = true;
        self.export_phase = None;
        self.export_result = None;
        self.export_timing = Some(export_timing::Timing::new(Instant::now()));
        self.progress = 0.;
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let mut backend = self.backend.clone();
        backend.job = m4b_editor_core::Job::new(cancel.clone(), Duration::from_secs(86400));
        let tx = self.tx.clone();
        let mut diagnostic = diagnostics::Job::export(&project, replacement.is_some());
        std::thread::spawn(move || {
            let mut progress = 0.;
            let result = (|| -> anyhow::Result<()> {
                let mut event = |event| {
                    if let ExportEvent::Progress(value) = event {
                        if value <= progress {
                            return;
                        }
                        progress = value;
                    }
                    diagnostic.export_event(&event, progress);
                    if !matches!(event, ExportEvent::Activity { .. }) {
                        let _ = tx.send(Message::Event(event, Instant::now()));
                    }
                };
                if let Some(replacement) = replacement {
                    let stage = replacement.staging_directory()?;
                    let staged = stage.path().join(format!("audio.{extension}"));
                    backend.export(&project, &staged, cancel.clone(), &mut |e| match e {
                        ExportEvent::Log(ref log) if log.starts_with("Saved ") => {}
                        ExportEvent::Progress(p) if p >= 1. => {}
                        _ => event(e),
                    })?;
                    replacement.publish(&staged, &cancel)?;
                    event(ExportEvent::Progress(1.));
                    event(ExportEvent::Log(format!("Saved {}", output.display())));
                } else {
                    backend.export(&project, &output, cancel.clone(), &mut event)?;
                }
                Ok(())
            })();
            let finished = Instant::now();
            diagnostic.finish(&result, cancel.load(Ordering::Relaxed));
            let result = result.map(|()| settings).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Message::Done(result, finished));
        });
        true
    }
}
impl App {
    fn render(&mut self, ctx: &egui::Context) {
        while let Ok(state) = self.playback.rx.try_recv() {
            self.preview = state;
        }
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Message::Copied(result) => {
                    self.busy = false;
                    match result {
                        Ok(path) => self.log(format!(
                            "{}: {}",
                            self.text("Project copied"),
                            path.display()
                        )),
                        Err(e) => self.log_error(e),
                    }
                }
                Message::Streams(index, result) => {
                    self.busy = false;
                    match result {
                        Ok(streams) => {
                            self.stream_track = Some(index);
                            self.stream_choices = streams;
                        }
                        Err(e) => self.log_error(e),
                    }
                }
                Message::Preview(directory, result) => {
                    self.busy = false;
                    self.exporting = false;
                    match result {
                        Ok(path) => {
                            self.encoded_preview = Some(directory);
                            self.preview_track = None;
                            self.playback.send(AudioCommand::Play(path, 0, None));
                        }
                        Err(e) => self.log_error(e),
                    }
                }
                Message::Cover(path, result) => {
                    if self.cover_path.as_ref() == Some(&path) {
                        match result {
                            Ok(image) => {
                                self.cover_texture =
                                    Some(ctx.load_texture("cover", image, Default::default()))
                            }
                            Err(e) => self.cover_error = e,
                        }
                    }
                }
                Message::RecentCover(path, image) => {
                    self.recent_loading = None;
                    if self.prefs.recent.contains(&path) {
                        let texture = image.map(|image| {
                            ctx.load_texture("recent cover", image, Default::default())
                        });
                        self.recent_covers.insert(path, texture);
                    }
                }
                Message::Encoders(result) => {
                    self.busy = false;
                    match result {
                        Ok(v) => self.bitrate_available = Some(v),
                        Err(e) => self.log_error(e),
                    }
                }
                Message::UpdateProgress(p) => self.progress = p,
                Message::UpdateChecked(channel, result) => {
                    self.checking = false;
                    if channel != self.update_channel {
                        continue;
                    }
                    match result {
                        Ok(update) => {
                            self.status = format!(
                                "{} {}",
                                self.text("Verified update"),
                                update.manifest.version
                            );
                            self.update_candidate = Some(update);
                        }
                        Err(error) => self.log_error(error),
                    }
                }
                Message::UpdateDownloaded(result) => {
                    self.busy = false;
                    match result {
                        Ok(download) => {
                            let playing = self.playback.is_busy();
                            let dirty = self.dirty;
                            if self.cancel.load(Ordering::Relaxed) {
                                self.log_error("Operation cancelled".into());
                                continue;
                            }
                            if let Err(error) = updater::guard(playing, dirty, true) {
                                self.log_error(error.to_string());
                                continue;
                            }
                            self.status = self.text("Preparing verified installer").into();
                            let tx = self.tx.clone();
                            self.busy = true;
                            std::thread::spawn(move || {
                                let _ = tx.send(Message::UpdateInstalled(
                                    download
                                        .install(playing, dirty)
                                        .map_err(|e| format!("{e:#}")),
                                ));
                            });
                        }
                        Err(error) => self.log_error(error),
                    }
                }
                Message::UpdateInstalled(result) => {
                    self.busy = false;
                    match result {
                        Ok(()) => {
                            self.stop_preview();
                            self.allow_close = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        Err(error) => self.log_error(error),
                    }
                }

                Message::LoadedProject(path, result) => match *result {
                    Ok((project, baseline)) => {
                        self.baseline = Some(baseline);
                        let _ = self
                            .tx
                            .send(Message::Loaded(Box::new(Ok(project)), Some(Some(path))));
                    }
                    Err(error) => {
                        self.busy = false;
                        self.log_error(error);
                    }
                },
                Message::ImportedM4b(result) => {
                    self.busy = false;
                    match *result {
                        Ok(project) if self.confirm_replace() => {
                            let _ = self
                                .tx
                                .send(Message::Loaded(Box::new(Ok(project)), Some(None)));
                        }
                        Ok(_) => {}
                        Err(error) => {
                            self.log(
                                self.text("Choose a readable MPEG-4 audiobook (.m4b).")
                                    .into(),
                            );
                            self.logs.push(error);
                        }
                    }
                }
                Message::Loaded(r, path) => {
                    self.busy = false;
                    self.import_progress = None;
                    match *r {
                        Ok(p) => {
                            self.stop_preview();
                            if path.is_some() {
                                self.clear_recovery();
                            }
                            self.project = p;
                            self.stream_track = None;
                            self.stream_choices.clear();
                            self.cover_path = None;
                            self.cover_texture = None;
                            self.project.settings.language = self.prefs.language.clone();
                            self.selected = true;
                            if path.is_some() {
                                self.section = Section::Project;
                            }
                            let clean = matches!(&path, Some(Some(_)));
                            if let Some(path) = path {
                                if path.is_none() {
                                    self.baseline = None;
                                }
                                self.path = path;
                            }
                            self.remember();
                            self.dirty = !clean;
                            diagnostics::project_loaded();
                            self.log(self.text("Loaded").into());
                        }
                        Err(e) => self.log_error(e),
                    }
                }
                Message::Event(event, now) => match event {
                    ExportEvent::Log(s) => self.log(s),
                    ExportEvent::Progress(p) => {
                        self.progress = p;
                        if self.exporting {
                            if let Some(timing) = &mut self.export_timing {
                                timing.progress(p, now);
                            }
                        }
                    }
                    ExportEvent::Phase(phase) => {
                        if self.exporting {
                            if let Some(timing) = &mut self.export_timing {
                                timing.phase(&phase, now);
                            }
                            self.export_phase = Some(phase);
                        }
                    }
                    ExportEvent::Activity { .. } => {}
                },
                Message::ImportProgress(phase) => self.import_progress = Some(phase),
                Message::Done(r, now) => {
                    if let Some(timing) = &mut self.export_timing {
                        timing.finish(now);
                    }
                    self.export_result = Some(r.as_ref().map(|_| ()).map_err(Clone::clone));
                    self.busy = false;
                    self.exporting = false;
                    match r {
                        Ok(settings) => {
                            self.dirty |= self.project.settings.format != settings.format
                                || self.project.settings.bitrate_kbps != settings.bitrate_kbps
                                || self.project.settings.lossless_remux != settings.lossless_remux;
                            self.project.settings = settings;
                            self.log(self.text("Export complete").into());
                        }
                        Err(e) => self.log_error(e),
                    }
                }
            }
        }
        if self.busy && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.cancel.store(true, Ordering::Relaxed);
            self.status = if self.exporting {
                self.text("Waiting for media job; export cancellation requested. Close again when finished.")
            } else {
                self.text("Waiting for background operation. Close again when finished.")
            }
            .into();
        }
        if !self.busy && !self.allow_close && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.confirm_replace() {
                self.clear_recovery();
                self.allow_close = true;
                self.dirty = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if !self.startup_checked && !self.busy && !self.playback.is_busy() {
            self.startup_checked = true;
            if updater::supported() {
                self.check_updates();
            }
        }
        if self.recovery.is_none() {
            self.recovery = self.pending_recoveries.pop().map(recovery::Prompt::new);
        }
        let recovery_open =
            self.recovery.is_some() && !self.busy && !self.creating && !self.allow_close;
        let modal_open = recovery_open
            || self.settings_open
            || self.remove_recent.is_some()
            || self.export_settings.is_some()
            || self.exporting
            || self.export_result.is_some();
        #[cfg(windows)]
        {
            if self
                .instance
                .as_ref()
                .is_some_and(instance::Primary::activate)
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            if !self.busy
                && !modal_open
                && !self.creating
                && !self.allow_close
                && self.update_candidate.is_none()
            {
                if let Some(path) = self.instance.as_ref().and_then(instance::Primary::project) {
                    let offer = rfd::MessageDialog::new()
                        .set_title(self.text("Open requested project?"))
                        .set_description(path.display().to_string())
                        .set_buttons(rfd::MessageButtons::YesNo)
                        .show();
                    if offer == rfd::MessageDialogResult::Yes && self.confirm_replace() {
                        self.load_path(path);
                    }
                }
            }
        }
        if self.update_candidate.is_some() && !self.busy && !self.playback.is_busy() && !modal_open
        {
            let version = self
                .update_candidate
                .as_ref()
                .unwrap()
                .manifest
                .version
                .clone();
            egui::Window::new(self.text("Update available"))
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(format!("{} → {}", env!("CARGO_PKG_VERSION"), version));
                    if ui.button(self.text("Later")).clicked() {
                        self.update_candidate = None;
                    }
                    if ui.button(self.text("Update")).clicked() {
                        if self.dirty {
                            self.save(false);
                        }
                        if updater::guard(self.busy || self.playback.is_busy(), self.dirty, true)
                            .is_ok()
                        {
                            if let Some(update) = self
                                .update_candidate
                                .take()
                                .filter(|u| u.matches_channel(self.update_channel))
                            {
                                self.busy = true;
                                self.progress = 0.;
                                self.cancel = Arc::new(AtomicBool::new(false));
                                let cancel = self.cancel.clone();
                                self.status = self.text("Downloading").into();
                                let tx = self.tx.clone();
                                std::thread::spawn(move || {
                                    let progress_tx = tx.clone();
                                    let result = update
                                        .download_progress(false, &cancel, |p| {
                                            let _ = progress_tx.send(Message::UpdateProgress(p));
                                        })
                                        .map_err(|e| format!("Update download: {e:#}"));
                                    let _ = tx.send(Message::UpdateDownloaded(result));
                                });
                            }
                        }
                    }
                });
        }
        if self.cover_path != self.project.cover {
            self.cover_path = self.project.cover.clone();
            self.cover_texture = None;
            self.cover_error.clear();
            if let Some(path) = self.cover_path.clone() {
                let tx = self.tx.clone();
                let backend = self.backend.clone();
                std::thread::spawn(move || {
                    let result = ui_media::load_cover(&backend, &path, 560)
                        .map_err(|e| format!("Unreadable cover / Couverture illisible: {e:#}"));
                    let _ = tx.send(Message::Cover(path, result));
                });
            }
        }
        // Follow OS theme when system is selected; egui polls native theme changes.
        ctx.set_theme(
            match self
                .prefs
                .theme
                .unwrap_or_else(|| preferences::Theme::from_project(&self.project.settings.theme))
            {
                preferences::Theme::Dark => egui::ThemePreference::Dark,
                preferences::Theme::Light => egui::ThemePreference::Light,
                preferences::Theme::System => egui::ThemePreference::System,
            },
        );
        if self.dirty && self.last_save.elapsed() > Duration::from_secs(15) {
            if let Err(e) = self.project.save(&self.recovery_store.slot()) {
                self.log_error(format!("{}: {e:#}", self.text("Autosave")));
            }
            self.last_save = Instant::now();
        }
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            if modal_open {
                ui.disable();
            }
            egui::menu::bar(ui, |ui| {
                ui.add_enabled_ui(!self.busy && !self.creating, |ui| {
                    ui.menu_button(self.text("Project"), |ui| {
                        if ui.button(self.text("New…")).clicked() {
                            if self.confirm_replace() {
                                self.creating = true;
                                self.new_name.clear();
                            }
                            ui.close_menu();
                        }
                        if ui.button(self.text("Open…")).clicked() {
                            self.open();
                            ui.close_menu();
                        }
                        if ui.button(self.text("Save")).clicked() {
                            self.save(false);
                            ui.close_menu();
                        }
                        if ui.button(self.text("Save as…")).clicked() {
                            self.save(true);
                            ui.close_menu();
                        }
                        if ui.button(self.text("Copy portable project…")).clicked() {
                            if let Some(parent) = rfd::FileDialog::new().pick_folder() {
                                let safe_name: String = self
                                    .project
                                    .name
                                    .chars()
                                    .map(|c| if c.is_alphanumeric() { c } else { '-' })
                                    .collect();
                                let destination = parent.join(format!("portable-{safe_name}"));
                                let project = self.project.clone();
                                let tx = self.tx.clone();
                                self.cancel = Arc::new(AtomicBool::new(false));
                                let job = m4b_editor_core::Job::new(
                                    self.cancel.clone(),
                                    Duration::from_secs(3600),
                                );
                                self.busy = true;
                                std::thread::spawn(move || {
                                    let _ = tx.send(Message::Copied(
                                        project
                                            .copy_to_with_job(&destination, &job)
                                            .map_err(|e| e.to_string()),
                                    ));
                                });
                            }
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button(self.text("Import…")).clicked() {
                            self.import_m4b();
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(
                                self.selected,
                                egui::Button::new(self.text("Export audio…")),
                            )
                            .clicked()
                        {
                            self.show_export_settings();
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui
                            .add_enabled(
                                self.selected,
                                egui::Button::new(self.text("Close project")),
                            )
                            .clicked()
                        {
                            self.close_project();
                            ui.close_menu();
                        }
                    });
                });
                if ui
                    .add_enabled(
                        !self.busy && !self.creating,
                        egui::Button::new(self.text("Settings")),
                    )
                    .clicked()
                {
                    self.settings_open = true;
                    ctx.memory_mut(|memory| {
                        if let Some(id) = memory.focused() {
                            memory.surrender_focus(id);
                        }
                    });
                }
                ui.menu_button(self.text("Help"), |ui| {
                    if ui.button(self.text("Export diagnostic log…")).clicked() {
                        ui.close_menu();
                        let choice = rfd::MessageDialog::new()
                            .set_title(self.text("Export diagnostic log…"))
                            .set_description(self.text("Export local job activity and error categories. You choose whether to share the file."))
                            .set_buttons(rfd::MessageButtons::OkCancel)
                            .show();
                        if choice == rfd::MessageDialogResult::Ok {
                            if let Some(path) = rfd::FileDialog::new().set_file_name("m4b-editor-diagnostics.log").save_file() {
                                match diagnostics::export(&path) {
                                    Ok(()) => self.log(self.text("Diagnostic log exported").into()),
                                    Err(error) => self.log_error(error.to_string()),
                                }
                            }
                        }
                    }
                    if ui.button(self.text("Open logs folder")).clicked() {
                        ui.close_menu();
                        if let Err(error) = diagnostics::open_folder() {
                            self.log_error(error.to_string());
                        }
                    }
                    if updater::supported() && ui
                        .add_enabled(
                            !self.checking && !self.busy && !self.playback.is_busy(),
                            egui::Button::new(self.text("Check for updates…")),
                        )
                        .clicked()
                    {
                        self.check_updates();
                        ui.close_menu();
                    } else if !updater::supported()
                        && ui.button(self.text("Open release downloads…")).clicked()
                    {
                        ctx.open_url(egui::OpenUrl::new_tab(
                            "https://github.com/aruel-perso/m4b-editor/releases/latest",
                        ));
                        ui.close_menu();
                    }
                });
                if self.busy && ui.button(self.text("Cancel operation")).clicked() {
                    self.cancel.store(true, Ordering::Relaxed);
                }
            });
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            if modal_open {
                ui.disable();
            }
            if let Some(phase) = self.import_progress {
                match phase {
                    ImportEvent::Inspecting => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(self.text("Inspecting input files…"));
                        });
                    }
                    ImportEvent::Extracting { done, total }
                    | ImportEvent::Probing { done, total }
                    | ImportEvent::ComparingCovers { done, total } => {
                        let label = match phase {
                            ImportEvent::Inspecting => unreachable!(),
                            ImportEvent::Extracting { .. } => "Extracting or copying audio…",
                            ImportEvent::Probing { .. } => "Analyzing audio…",
                            ImportEvent::ComparingCovers { .. } => "Comparing embedded covers…",
                        };
                        ui.add(
                            egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                                .text(format!("{} {done}/{total}", self.text(label))),
                        );
                    }
                }
            } else if self.busy && !self.exporting && self.progress > 0. {
                ui.add(egui::ProgressBar::new(self.progress).show_percentage());
            }
            ui.label(&self.status);
            egui::CollapsingHeader::new(self.text("Logs / errors")).show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(140.)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for log in &self.logs {
                            ui.label(log);
                        }
                    });
            });
        });
        if !self.busy && (self.preview.active || self.preview.paused) {
            egui::TopBottomPanel::bottom("transport").show(ctx, |ui| {
                if modal_open {
                    ui.disable();
                }
                ui.horizontal(|ui| {
                    if ui.button("⏮").clicked() {
                        self.navigate_preview(false);
                    }
                    if ui
                        .button(if self.preview.paused { "▶" } else { "⏸" })
                        .clicked()
                    {
                        self.playback
                            .send(AudioCommand::Pause(!self.preview.paused));
                    }
                    if ui.button("⏹").clicked() {
                        self.stop_preview();
                    }
                    if ui.button("⏭").clicked() {
                        self.navigate_preview(true);
                    }
                    if let Some(index) = self.preview_track {
                        if let Some(track) = self.project.tracks.get(index) {
                            let title = if self.project.tracks.len() == 1 {
                                self.project
                                    .chapters
                                    .iter()
                                    .rev()
                                    .find(|c| c.start_ms <= self.preview.position_ms)
                                    .map(|c| c.title.clone())
                            } else {
                                None
                            }
                            .unwrap_or_else(|| {
                                track
                                    .path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned()
                            });
                            ui.label(title);
                            ui.label(format!(
                                "{} / {}",
                                playback::clock(self.preview.position_ms),
                                playback::clock(track.duration_ms)
                            ));
                            let response = ui.add(
                                egui::Slider::new(&mut self.seek_ms, 0..=track.duration_ms)
                                    .show_value(false),
                            );
                            if response.drag_stopped()
                                || (response.changed() && !response.dragged())
                            {
                                self.play_preview(index, self.seek_ms);
                            } else if !response.dragged() {
                                self.seek_ms = self.preview.position_ms;
                            }
                        }
                    }
                });
            });
        }
        if self.creating && !modal_open {
            egui::Window::new(self.text("New project"))
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(self.text("Project name"));
                    ui.text_edit_singleline(&mut self.new_name);
                    if ui
                        .add_enabled(
                            !self.new_name.trim().is_empty(),
                            egui::Button::new(self.text("Create…")),
                        )
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Project", &["m4bproj"])
                            .set_file_name("book.m4bproj")
                            .save_file()
                            .and_then(|p| {
                                let target = project_save_target(&p);
                                let exists = target.exists();
                                let consent = !exists
                                    || rfd::MessageDialog::new()
                                        .set_title(self.text("Overwrite project"))
                                        .set_description(format!(
                                            "{} {} ?",
                                            self.text("Replace"),
                                            target.display()
                                        ))
                                        .set_buttons(rfd::MessageButtons::YesNo)
                                        .show()
                                        == rfd::MessageDialogResult::Yes;
                                overwrite_allowed(exists, consent).then_some(target)
                            })
                        {
                            let mut project =
                                Project::named(&self.new_name).expect("validated project name");
                            project.settings = self.project.settings.clone();
                            project.settings.bitrate_kbps = 160;
                            project.settings.language = self.prefs.language.clone();
                            match project.save_new(&path) {
                                Ok(()) => {
                                    self.stop_preview();
                                    self.clear_recovery();
                                    self.project = project;
                                    self.baseline = None;
                                    self.stream_track = None;
                                    self.stream_choices.clear();
                                    self.selected = true;
                                    self.recent_covers.remove(&path);
                                    self.path = Some(path);
                                    self.remember();
                                    self.dirty = false;
                                    self.creating = false;
                                    self.section = Section::Project;
                                }
                                Err(e) => {
                                    self.log(format!("{}: {e:#}", self.text("Create project")))
                                }
                            }
                        }
                    }
                    if ui.button(self.text("Cancel")).clicked() {
                        self.creating = false;
                    }
                });
        }
        if !self.selected {
            egui::CentralPanel::default().show(ctx, |ui| {
                let heading = self.text("Welcome to M4B Editor");
                let font = egui::TextStyle::Heading.resolve(ui.style());
                let galley = ui.painter().layout(
                    heading.into(),
                    font,
                    ui.visuals().text_color(),
                    ui.available_width(),
                );
                let center = ui.available_rect_before_wrap().center();
                let rect = egui::Rect::from_center_size(center, galley.size());
                ui.painter()
                    .galley(rect.min, galley, ui.visuals().text_color());
                ui.add_enabled_ui(!self.busy && !self.creating && !modal_open, |ui| {
                    if ui.button(self.text("Create project…")).clicked() {
                        self.creating = true;
                    }
                    if ui.button(self.text("Open project…")).clicked() {
                        self.open();
                    }
                    ui.heading(self.text("Recent projects"));
                    if !self.busy && !modal_open {
                        self.queue_recent_cover();
                    }
                    let recent_height = (rect.top() - ui.cursor().top() - 8.).max(0.);
                    egui::ScrollArea::vertical()
                        .max_height(recent_height)
                        .show(ui, |ui| {
                            for path in self.prefs.recent.clone() {
                                ui.horizontal(|ui| {
                                    let size = egui::vec2(48., 48.);
                                    let (rect, _) =
                                        ui.allocate_exact_size(size, egui::Sense::hover());
                                    if let Some(Some(texture)) = self.recent_covers.get(&path) {
                                        let original = texture.size_vec2();
                                        let image_size =
                                            original * (48. / original.x.max(original.y));
                                        ui.painter().image(
                                            texture.id(),
                                            egui::Rect::from_center_size(rect.center(), image_size),
                                            egui::Rect::from_min_max(
                                                egui::Pos2::ZERO,
                                                egui::pos2(1., 1.),
                                            ),
                                            egui::Color32::WHITE,
                                        );
                                    } else {
                                        ui.painter().rect_filled(
                                            rect,
                                            3.,
                                            ui.visuals().faint_bg_color,
                                        );
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .small_button("×")
                                                .on_hover_text(self.text("Remove from recents"))
                                                .clicked()
                                            {
                                                self.remove_recent = Some(path.clone());
                                            }
                                            let label = path.display().to_string();
                                            if ui
                                                .add_enabled(
                                                    path.is_file(),
                                                    egui::Button::new(&label)
                                                        .wrap_mode(egui::TextWrapMode::Truncate),
                                                )
                                                .on_hover_text(&label)
                                                .clicked()
                                            {
                                                self.load_path(path);
                                            }
                                        },
                                    );
                                });
                            }
                        });
                });
            });
            self.finish_frame(ctx, recovery_open);
            return;
        }
        let sidebar = egui::SidePanel::left("navigation")
            .resizable(!modal_open)
            .default_width(self.prefs.sidebar)
            .width_range(120.0..=400.0)
            .show(ctx, |ui| {
                if modal_open {
                    ui.disable();
                }
                for (section, en) in [
                    (Section::Project, "Project"),
                    (Section::Metadata, "Metadata"),
                    (Section::Chapters, "Chapters"),
                ] {
                    let label = self.text(en);
                    ui.selectable_value(&mut self.section, section, label);
                }
            });
        if !modal_open {
            self.prefs.sidebar = sidebar.response.rect.width().clamp(120., 400.);
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_enabled_ui(!self.busy && !self.creating && !modal_open, |ui| {
                    let before = format!("{:?}", self.project);
                    let fr = self.prefs.language == "fr";
                    if self.section == Section::Metadata {
                        ui.heading(self.text("Book metadata"));
                        egui::Grid::new("metadata").num_columns(2).show(ui, |ui| {
                            for (label, value) in [
                                (catalog::text(fr, "Title"), &mut self.project.metadata.title),
                                (
                                    catalog::text(fr, "Author"),
                                    &mut self.project.metadata.author,
                                ),
                                (
                                    catalog::text(fr, "Narrator"),
                                    &mut self.project.metadata.narrator,
                                ),
                            ] {
                                ui.label(label);
                                ui.text_edit_singleline(value);
                                ui.end_row();
                            }
                            for (label, value) in [
                                (
                                    catalog::text(fr, "Publisher"),
                                    &mut self.project.metadata.publisher,
                                ),
                                (
                                    catalog::text(fr, "Book language (ISO 639-2)"),
                                    &mut self.project.metadata.book_language,
                                ),
                                ("Genre", &mut self.project.metadata.genre),
                                ("Date", &mut self.project.metadata.date),
                                (
                                    catalog::text(fr, "Copyright"),
                                    &mut self.project.metadata.copyright,
                                ),
                            ] {
                                ui.label(label);
                                ui.text_edit_singleline(value);
                                ui.end_row();
                            }
                            ui.label(self.text("Description"));
                            ui.text_edit_multiline(&mut self.project.metadata.description);
                            ui.end_row();
                        });
                        ui.horizontal(|ui| {
                            if ui.button(self.text("Choose cover…")).clicked() {
                                if let Some(p) = rfd::FileDialog::new()
                                    .add_filter(self.text("Image"), &["jpg", "jpeg", "png"])
                                    .pick_file()
                                {
                                    self.project.cover = Some(p);
                                    self.cover_path = None;
                                    self.cover_texture = None;
                                }
                            }
                            if ui.button(self.text("Remove cover")).clicked() {
                                self.project.cover = None;
                            }
                            ui.label(
                                self.project
                                    .cover
                                    .as_ref()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_else(|| self.text("No cover").into()),
                            );
                        });
                        if let Some(texture) = &self.cover_texture {
                            ui.add(
                                egui::Image::new(texture)
                                    .max_size(egui::vec2(280., 280.))
                                    .maintain_aspect_ratio(true),
                            );
                        } else if self.project.cover.is_none() {
                            ui.label(self.text("No cover"));
                        } else if !self.cover_error.is_empty() {
                            ui.label(self.text("Cover unavailable"))
                                .on_hover_text(&self.cover_error);
                        } else {
                            ui.spinner();
                        }
                        ui.separator();
                    }
                    if self.section == Section::Project {
                        ui.horizontal(|ui| {
                            ui.label(catalog::text(fr, "Archive password (not saved)"));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.archive_password)
                                    .password(true),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label(catalog::text(fr, "Absolute audio stream index"));
                            if ui
                                .button(catalog::text(fr, "Inspect audio streams"))
                                .clicked()
                            {
                                if let Some(index) = self.preview_track {
                                    let path = self.project.tracks[index].path.clone();
                                    let mut backend = self.backend.clone();
                                    let tx = self.tx.clone();
                                    self.cancel = Arc::new(AtomicBool::new(false));
                                    backend.job = m4b_editor_core::Job::new(
                                        self.cancel.clone(),
                                        Duration::from_secs(120),
                                    );
                                    self.busy = true;
                                    std::thread::spawn(move || {
                                        let _ = tx.send(Message::Streams(
                                            index,
                                            backend.audio_streams(&path).map_err(|e| e.to_string()),
                                        ));
                                    });
                                }
                            }
                            egui::ComboBox::from_id_salt("audio-stream")
                                .selected_text(self.selected_stream.to_string())
                                .show_ui(ui, |ui| {
                                    for stream in &self.stream_choices {
                                        ui.selectable_value(
                                            &mut self.selected_stream,
                                            stream.index,
                                            format!(
                                                "{}: {} · {} Hz · {} ch · {}",
                                                stream.index,
                                                stream.codec,
                                                stream.sample_rate,
                                                stream.channels,
                                                stream.language
                                            ),
                                        );
                                    }
                                });
                            if ui
                                .button(catalog::text(fr, "Apply selected stream"))
                                .clicked()
                            {
                                if let Some(index) = self.stream_track {
                                    let mut project = self.project.clone();
                                    let mut backend = self.backend.clone();
                                    self.cancel = Arc::new(AtomicBool::new(false));
                                    backend.job = m4b_editor_core::Job::new(
                                        self.cancel.clone(),
                                        Duration::from_secs(120),
                                    );
                                    let selected = self.selected_stream;
                                    let tx = self.tx.clone();
                                    self.busy = true;
                                    self.stop_preview();
                                    std::thread::spawn(move || {
                                        let result = project
                                            .select_stream(&backend, index, selected)
                                            .map(|_| project)
                                            .map_err(|e| e.to_string());
                                        let _ = tx.send(Message::Loaded(Box::new(result), None));
                                    });
                                }
                            }
                        });
                        if ui.button(self.text("Import audio / archives…")).clicked() {
                            self.import();
                        }
                        ui.heading(self.text("Tracks (custom chapters preserved)"));
                        ui.group(|ui| {
                            ui.heading(self.text("Pre-listening"));
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(
                                        self.preview.active,
                                        egui::Button::new(if self.preview.paused {
                                            self.text("Resume")
                                        } else {
                                            self.text("Pause")
                                        }),
                                    )
                                    .clicked()
                                {
                                    self.playback
                                        .send(AudioCommand::Pause(!self.preview.paused));
                                }
                                if ui.button(self.text("Stop")).clicked() {
                                    self.stop_preview();
                                }
                                let label = self.text("Volume");
                                if ui
                                    .add(egui::Slider::new(&mut self.volume, 0.0..=1.0).text(label))
                                    .changed()
                                {
                                    self.playback.send(AudioCommand::Volume(self.volume));
                                }
                            });
                            if !self.preview.message.is_empty() {
                                ui.label(self.text("Playback details"))
                                    .on_hover_text(&self.preview.message);
                            }
                        });
                        let mut play_track = None;
                        let mut movement = None;
                        let mut remove = None;
                        for (i, t) in self.project.tracks.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if track_move_button(
                                    ui,
                                    i > 0,
                                    "▲",
                                    catalog::text(fr, "Move track up"),
                                ) {
                                    movement = Some((i, i - 1));
                                }
                                if track_move_button(
                                    ui,
                                    i + 1 < self.project.tracks.len(),
                                    "▼",
                                    catalog::text(fr, "Move track down"),
                                ) {
                                    movement = Some((i, i + 1));
                                }
                                if ui.small_button("×").clicked() {
                                    remove = Some(i);
                                }
                                if ui.small_button(catalog::text(fr, "Play")).clicked() {
                                    play_track = Some(i);
                                }
                                ui.label(format!(
                                    "{} · {:.2}s · {} · {} Hz · {} ch",
                                    t.path.file_name().unwrap_or_default().to_string_lossy(),
                                    t.duration_ms as f64 / 1000.,
                                    t.codec,
                                    t.sample_rate,
                                    t.channels
                                ));
                            });
                        }
                        if let Some(index) = play_track {
                            self.play_preview(index, 0);
                        }
                        if let Some((a, b)) = movement {
                            self.stop_preview();
                            self.stream_track = None;
                            self.stream_choices.clear();
                            let mut order: Vec<_> = (0..self.project.tracks.len()).collect();
                            order.swap(a, b);
                            if let Err(e) = self.project.reorder_tracks(&order) {
                                self.log(e.to_string());
                            }
                        }
                        if let Some(i) = remove {
                            self.stop_preview();
                            self.stream_track = None;
                            self.stream_choices.clear();
                            if let Err(e) = self.project.remove_track(i) {
                                self.log(e.to_string());
                            }
                        }
                        ui.separator();
                    }
                    if self.section == Section::Chapters {
                        ui.heading(self.text("Chapters — start in milliseconds"));
                        let mut chapter_jump = None;
                        let mut delete = None;
                        for (i, c) in self.project.chapters.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.small_button(catalog::text(fr, "Play")).clicked() {
                                    chapter_jump = Some(c.start_ms);
                                }
                                ui.label(format!("{}", i + 1));
                                ui.add(egui::DragValue::new(&mut c.start_ms).speed(100));
                                ui.text_edit_singleline(&mut c.title);
                                if ui.small_button("×").clicked() {
                                    delete = Some(i);
                                }
                            });
                        }
                        if let Some(ms) = chapter_jump {
                            if let Some((index, offset)) = playback::locate(
                                self.project.tracks.iter().map(|t| t.duration_ms),
                                ms,
                            ) {
                                self.play_preview(index, offset);
                            }
                        }
                        if let Some(i) = delete {
                            self.project.chapters.remove(i);
                        }
                        ui.horizontal(|ui| {
                            if ui.button(self.text("Add chapter")).clicked() {
                                let start = self
                                    .project
                                    .chapters
                                    .last()
                                    .map(|c| c.start_ms.saturating_add(1000))
                                    .unwrap_or(0);
                                self.project.chapters.push(Chapter {
                                    start_ms: start,
                                    title: format!(
                                        "{} {}",
                                        self.text("Chapter"),
                                        self.project.chapters.len() + 1
                                    ),
                                });
                            }
                            if ui.button(self.text("One per track")).clicked() {
                                self.project.reset_chapters();
                            }
                        });
                        ui.separator();
                    }
                    if before != format!("{:?}", self.project) {
                        self.dirty = true;
                    }
                });
            });
        });
        self.finish_frame(ctx, recovery_open);
    }
}
fn track_move_button(ui: &mut egui::Ui, enabled: bool, icon: &str, label: &str) -> bool {
    let response = ui.add_enabled(enabled, egui::Button::new(icon));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response
        .on_hover_text(label)
        .on_disabled_hover_text(label)
        .clicked()
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.render(ctx);
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.prefs.save(&preferences_path());
        self.playback.send(AudioCommand::Exit);
        self.cancel.store(true, Ordering::Relaxed);
        if self.dirty && !self.allow_close {
            self.recovery_store.preserve_on_exit();
            if let Err(error) = self.project.save(&self.recovery_store.slot()) {
                self.log_error(error.to_string());
            }
        }
        diagnostics::shutdown();
    }
}

fn modal(ctx: &egui::Context, id: &str, contents: impl FnOnce(&mut egui::Ui)) {
    let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new(id));
    egui::Area::new(layer.id)
        .order(layer.order)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .movable(false)
        .show(ctx, |ui| {
            let screen = ctx.screen_rect();
            ui.set_clip_rect(screen);
            ui.painter()
                .rect_filled(screen, 0., egui::Color32::from_black_alpha(120));
            egui::Frame::window(ui.style()).show(ui, |ui| {
                ui.set_max_width(420.);
                contents(ui);
            });
        });
    // Let child popup areas stay above this modal; raising it after rendering
    // would cover dropdown entries and route their clicks to the parent.
}

fn project_save_target(path: &std::path::Path) -> PathBuf {
    path.with_extension("m4bproj")
}
fn overwrite_allowed(exists: bool, consent: bool) -> bool {
    !exists || consent
}

// A cancelled/failed save must never authorize destructive navigation.
fn replacement_allowed(choice: rfd::MessageDialogResult, saved: bool) -> bool {
    choice == rfd::MessageDialogResult::No || (choice == rfd::MessageDialogResult::Yes && saved)
}
#[cfg(test)]
mod project_ui_tests {
    use super::*;
    #[test]
    fn menu_import_requires_m4b_extension_and_content() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("renamed.m4b");
        std::fs::write(&source, b"RIFF\x24\0\0\0WAVEfmt ").unwrap();
        assert!(ui_media::check_m4b_header(&source).is_err());
        let header = b"\0\0\0\x18ftypM4A \0\0\0\0M4A isom";
        std::fs::write(&source, header).unwrap();
        assert!(ui_media::check_m4b_header(&source).is_ok());
        // M4B is an audiobook extension over ISO BMFF, not an exclusive Apple brand.
        let mut generic = header.to_vec();
        generic[8..12].copy_from_slice(b"isom");
        std::fs::write(&source, &generic).unwrap();
        assert!(ui_media::check_m4b_header(&source).is_ok());
        let wrong_extension = dir.path().join("book.m4a");
        std::fs::write(&wrong_extension, header).unwrap();
        assert!(ui_media::check_m4b_header(&wrong_extension).is_err());
        assert!(ui_media::open_m4b(&DesktopFfmpeg::default(), &source).is_err());
        let backend = DesktopFfmpeg::default();
        if backend.ffprobe.is_file()
            || std::process::Command::new(&backend.ffprobe)
                .arg("-version")
                .output()
                .is_ok_and(|out| out.status.success())
        {
            let source = dir.path().join("synthetic.M4B");
            std::fs::write(
                &source,
                include_bytes!("../../../tests/fixtures/tagged.m4b"),
            )
            .unwrap();
            let project = ui_media::open_m4b(&backend, &source).unwrap();
            assert_eq!(project.tracks[0].codec, "aac");
        }
    }

    #[test]
    fn recent_cover_uses_relative_metadata_and_fails_gracefully() {
        let dir = tempfile::tempdir().unwrap();
        let project_path = dir.path().join("book.m4bproj");
        let cover_path = dir.path().join("cover.png");
        let mut project = Project::named("Synthetic").unwrap();
        project.save(&project_path).unwrap();
        let backend = DesktopFfmpeg::default();
        assert!(ui_media::recent_cover(&backend, &project_path).is_none());
        project.cover = Some("cover.png".into());
        project.save(&project_path).unwrap();
        assert!(ui_media::recent_cover(&backend, &project_path).is_none());
        std::fs::write(&cover_path, b"bad cover").unwrap();
        assert!(ui_media::recent_cover(&backend, &project_path).is_none());
        image::RgbaImage::from_pixel(160, 80, image::Rgba([255, 0, 0, 255]))
            .save(&cover_path)
            .unwrap();
        assert_eq!(
            ui_media::recent_cover(&backend, &project_path)
                .unwrap()
                .size,
            [96, 48]
        );
    }

    #[test]
    fn recent_removal_persists_only_after_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("book.m4bproj");
        let cover = dir.path().join("cover.png");
        let prefs = dir.path().join("preferences.json");
        std::fs::write(&project, b"project").unwrap();
        std::fs::write(&cover, b"cover").unwrap();
        let mut app = App::default();
        app.prefs.recent = vec![project.clone()];
        app.prefs.save(&prefs).unwrap();
        app.remove_recent = Some(project.clone());
        app.finish_remove_recent(false, &prefs);
        assert_eq!(
            preferences::Preferences::load(&prefs).recent.as_slice(),
            std::slice::from_ref(&project)
        );
        app.remove_recent = Some(project.clone());
        app.finish_remove_recent(true, &prefs);
        assert!(preferences::Preferences::load(&prefs).recent.is_empty());
        assert_eq!(std::fs::read(project).unwrap(), b"project");
        assert_eq!(std::fs::read(cover).unwrap(), b"cover");
    }

    #[test]
    fn missing_recent_cover_is_cached_across_redraws() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.m4bproj");
        let mut app = App {
            startup_checked: true,
            pending_recoveries: Vec::new(),
            ..App::default()
        };
        app.prefs.recent = vec![path.clone()];
        app.recent_loading = Some(path.clone());
        app.tx
            .send(Message::RecentCover(path.clone(), None))
            .unwrap();
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
            assert!(matches!(app.recent_covers.get(&path), Some(None)));
            assert!(app.recent_loading.is_none());
        }
    }

    #[test]
    fn modal_dropdown_remains_above_parent_and_accepts_selection() {
        let ctx = egui::Context::default();
        let mut selected = 160;
        let mut render = |events: Vec<egui::Event>| {
            let mut combo = egui::Rect::NOTHING;
            let mut option = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000., 800.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    modal(ctx, "dropdown-regression", |ui| {
                        combo = egui::ComboBox::from_id_salt("rate")
                            .selected_text(selected.to_string())
                            .show_ui(ui, |ui| {
                                option = Some(ui.selectable_value(&mut selected, 128, "128").rect);
                            })
                            .response
                            .rect;
                        ui.label("Parent content below the dropdown");
                        ui.add_space(200.);
                    });
                },
            );
            (combo, option)
        };
        let click = |pos| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        for _ in 0..3 {
            render(vec![]);
        }
        let (combo, _) = render(vec![]);
        render(click(combo.center()));
        let (_, option) = render(vec![]);
        render(click(option.expect("dropdown opened").center()));
        render(vec![]);
        assert_eq!(selected, 128);
    }

    #[test]
    fn export_draft_is_independent_and_bitrate_matches_encoding() {
        let mut app = App::default();
        let before = format!("{:?}", app.project);
        app.show_export_settings();
        let draft = app.export_settings.as_mut().unwrap();
        draft.format = ExportFormat::Flac;
        draft.bitrate_kbps = 48;
        assert!(ui_media::bitrates(draft.format).is_empty());
        assert_eq!(
            ui_media::bitrates(ExportFormat::Mp3),
            [48, 64, 96, 128, 160, 192, 256, 320]
        );
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| app.show_export(ctx));
        assert!(app.export_settings.is_none());
        assert_eq!(format!("{:?}", app.project), before);
        assert!(!app.dirty && !app.busy);
    }

    #[test]
    fn overwrite_requires_original_destination_and_protects_sources() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("book.m4b");
        let staged = dir.path().join("staged.m4b");
        std::fs::write(&destination, b"original").unwrap();
        std::fs::write(&staged, b"encoded").unwrap();
        assert!(ui_media::Replacement::prepare(&destination, [&destination]).is_err());
        let replacement = ui_media::Replacement::prepare(&destination, []).unwrap();
        std::fs::write(&destination, b"changed size").unwrap();
        assert!(replacement
            .publish(&staged, &Arc::new(AtomicBool::new(false)))
            .is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"changed size");
        let replacement = ui_media::Replacement::prepare(&destination, []).unwrap();
        assert!(replacement
            .publish(&staged, &Arc::new(AtomicBool::new(true)))
            .is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"changed size");
        let replacement = ui_media::Replacement::prepare(&destination, []).unwrap();
        replacement
            .publish(&staged, &Arc::new(AtomicBool::new(false)))
            .unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), b"encoded");
    }

    #[test]
    fn normalized_save_target_requires_overwrite_consent() {
        for input in ["book", "book.json", "book.m4bproj", "book.M4BPROJ"] {
            assert_eq!(
                project_save_target(std::path::Path::new(input)),
                PathBuf::from("book.m4bproj")
            );
        }
        assert!(!overwrite_allowed(true, false));
        assert!(overwrite_allowed(true, true));
        assert!(overwrite_allowed(false, false));
    }
    #[test]
    fn welcome_requires_explicit_selection() {
        let app = App::default();
        assert!(!app.selected);
        assert!(app.path.is_none());
        assert!(!app.startup_checked);
    }
    #[test]
    fn cover_decode_limits_and_absent_state() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("cover.png");
        let image = image::RgbaImage::from_pixel(80, 40, image::Rgba([255, 0, 0, 255]));
        image.save(&path).unwrap();
        let decoded = ui_media::load_cover(&DesktopFfmpeg::default(), &path, 560).unwrap();
        assert_eq!(decoded.size, [80, 40]);
        std::fs::write(&path, b"invalid image").unwrap();
        assert!(ui_media::load_cover(&DesktopFfmpeg::default(), &path, 560).is_err());
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(16 * 1024 * 1024 + 1).unwrap();
        assert!(ui_media::load_cover(&DesktopFfmpeg::default(), &path, 560).is_err());
    }
    #[test]
    fn unsaved_navigation_requires_discard_or_successful_save() {
        assert!(replacement_allowed(rfd::MessageDialogResult::No, false));
        assert!(replacement_allowed(rfd::MessageDialogResult::Yes, true));
        assert!(!replacement_allowed(rfd::MessageDialogResult::Yes, false));
        assert!(!replacement_allowed(rfd::MessageDialogResult::Cancel, true));
        assert!(!replacement_allowed(
            rfd::MessageDialogResult::Cancel,
            false
        ));
    }
}

#[cfg(test)]
mod release040_ui_tests {
    use super::*;
    fn title_commands(output: &egui::FullOutput) -> Vec<&str> {
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .filter_map(|command| match command {
                egui::ViewportCommand::Title(title) => Some(title.as_str()),
                _ => None,
            })
            .collect()
    }

    fn text_shape(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.visual_bounding_rect())
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_shape(shape, label)),
            _ => None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn temporary_state_rejects_public_directories_and_symlinks() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let temp = tempfile::tempdir().unwrap();
        let path = private_unix_state(temp.path()).unwrap();
        assert_eq!(private_unix_state(temp.path()).unwrap(), path);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_unix_state(temp.path()).is_err());
        std::fs::remove_dir(&path).unwrap();
        symlink(temp.path(), &path).unwrap();
        assert!(private_unix_state(temp.path()).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn startup_skips_unsupported_updater() {
        let mut app = App::default();
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
        assert!(app.startup_checked && !app.checking);
    }

    #[test]
    fn welcome_heading_is_centered_in_both_languages_when_resized() {
        let mut app = App {
            startup_checked: true,
            ..App::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("recent.m4bproj");
        Project::named("Recent").unwrap().save(&project).unwrap();
        app.prefs.recent = vec![project.clone()];
        app.recent_covers.insert(project.clone(), None);
        let ctx = egui::Context::default();
        for (language, label) in [
            ("en", "Welcome to M4B Editor"),
            ("fr", "Bienvenue dans M4B Editor"),
        ] {
            app.prefs.language = language.into();
            for size in [egui::vec2(560., 480.), egui::vec2(1040., 700.)] {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| app.render(ctx),
                );
                let visible_text = |label| {
                    output
                        .shapes
                        .iter()
                        .find_map(|shape| {
                            text_shape(&shape.shape, label)
                                .filter(|rect| shape.clip_rect.contains_rect(*rect))
                        })
                        .unwrap_or_else(|| panic!("Expected visible text: {label}"))
                };
                let heading = visible_text(label);
                for action in ["Create project…", "Open project…", "Recent projects"] {
                    assert!(visible_text(app.text(action)).bottom() < heading.top());
                }
                assert!(visible_text(&project.display().to_string()).bottom() < heading.top());
                let center = heading.center();
                assert!((center.x - size.x / 2.).abs() < 4., "{center:?} {size:?}");
                // The central panel may sit below the menu bar.
                assert!((center.y - size.y / 2.).abs() < 12., "{center:?} {size:?}");
            }
        }
    }

    #[test]
    fn successful_save_clears_autosave_and_close_leaves_project_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("saved.m4bproj");
        let mut app = App {
            project: Project::named("Edited").unwrap(),
            path: Some(path.clone()),
            dirty: true,
            selected: true,
            ..App::default()
        };
        let slot = app.recovery_store.slot();
        app.project.save(&slot).unwrap();
        app.save(false);
        assert!(!app.dirty);
        assert!(!slot.exists());
        app.close_project();
        assert!(!app.selected);
        assert_eq!(Project::load(&path).unwrap().name, "Edited");
    }

    #[test]
    fn export_end_preserves_elapsed_for_success_cancellation_and_failure() {
        let started = Instant::now() - Duration::from_secs(4);
        let elapsed = Duration::from_secs(7);
        let ctx = egui::Context::default();
        for result in [
            Ok(Settings::default()),
            Err("Export cancelled".into()),
            Err("Synthetic export failure".into()),
        ] {
            let mut app = App {
                startup_checked: true,
                busy: true,
                exporting: true,
                export_timing: Some(export_timing::Timing::new(started)),
                ..App::default()
            };
            app.prefs.language = "en".into();
            let frame = |app: &mut App| {
                // Area performs an invisible sizing pass when it first opens.
                let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
                ctx.run(egui::RawInput::default(), |ctx| app.render(ctx))
            };
            let shows = |output: &egui::FullOutput, label: &str| {
                assert!(
                    output
                        .shapes
                        .iter()
                        .any(|shape| text_shape(&shape.shape, label).is_some()),
                    "{label}"
                );
            };
            let running = frame(&mut app);
            let running_label = "Elapsed / estimated total ";
            assert!(running.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text)
                    if text.galley.text().starts_with(running_label)
                        && text.galley.text().ends_with(" / Unknown"))
            }));
            let timing = app.export_timing.as_mut().unwrap();
            timing.phase(&ExportPhase::Encoding, started);
            timing.progress(0.25, started + Duration::from_secs(3));
            let estimated = frame(&mut app);
            assert!(estimated.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text)
                    if text.galley.text().starts_with(running_label)
                        && text.galley.text().contains(" / ~"))
            }));
            app.tx
                .send(Message::Done(result, started + elapsed))
                .unwrap();
            shows(&frame(&mut app), "Elapsed 00:07");
            assert!(!app.exporting);
            let timing = app.export_timing.as_ref().unwrap();
            assert_eq!(timing.elapsed(started + elapsed * 2), elapsed);
            assert!(timing.estimate(started + elapsed * 2).is_none());
        }
    }

    #[test]
    fn export_modal_retains_terminal_error_until_dismissed() {
        let mut app = App {
            startup_checked: true,
            busy: true,
            exporting: true,
            ..App::default()
        };
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
        assert!(app.export_result.is_none());
        app.tx
            .send(Message::Event(ExportEvent::Progress(0.63), Instant::now()))
            .unwrap();
        app.tx
            .send(Message::Done(
                Err("Synthetic encoder failure".into()),
                Instant::now(),
            ))
            .unwrap();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
        assert_eq!(app.progress, 0.63);
        assert!(!app.busy && !app.exporting);
        assert_eq!(
            app.export_result.as_ref().unwrap().as_ref().unwrap_err(),
            "Synthetic encoder failure"
        );
        let escape = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = ctx.run(
            egui::RawInput {
                events: vec![escape],
                ..Default::default()
            },
            |ctx| app.render(ctx),
        );
        assert!(app.export_result.is_none());
    }

    #[test]
    fn native_title_follows_loaded_and_closed_project() {
        let mut app = App {
            startup_checked: true,
            ..App::default()
        };
        let ctx = egui::Context::default();
        assert_eq!(desktop_viewport().title.as_deref(), Some(APP_TITLE));
        let output = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
        assert!(title_commands(&output).is_empty());

        let project = Project::named("L’Étranger").unwrap();
        let expected = format!("{APP_TITLE} - {}", project.name);
        app.tx
            .send(Message::Loaded(Box::new(Ok(project)), None))
            .unwrap();
        let output = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
        assert_eq!(title_commands(&output), vec![expected.as_str()]);

        app.busy = true;
        app.close_project();
        assert!(app.selected);
        app.busy = false;
        app.dirty = false;
        app.close_project();
        let output = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
        assert_eq!(title_commands(&output), vec![APP_TITLE]);
    }

    #[test]
    fn recovery_queue_restores_as_unsaved_and_persists_dismissal() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.m4bproj");
        let second = dir.path().join("second.m4bproj");
        let original = dir.path().join("original.m4bproj");
        for (path, name) in [
            (&first, "Recovered"),
            (&second, "Dismissed"),
            (&original, "Original"),
        ] {
            Project::named(name).unwrap().save_new(path).unwrap();
        }
        let snapshots = [&first, &second, &original].map(|path| std::fs::read(path).unwrap());
        let mut app = App {
            startup_checked: true,
            pending_recoveries: vec![second.clone(), first.clone()],
            selected: true,
            path: Some(original.clone()),
            project: Project::load(&original).unwrap(),
            ..App::default()
        };
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
        assert_eq!(
            app.recovery.as_ref().unwrap().name.as_deref().unwrap(),
            "Recovered"
        );
        assert!(app.recovery.as_ref().unwrap().date.is_some());
        app.busy = true;
        app.restore_recovery();
        assert_eq!(app.path.as_ref(), Some(&original));
        app.busy = false;
        app.restore_recovery();
        assert_eq!(app.project.name, "Recovered");
        assert!(app.selected && app.dirty && app.path.is_none() && app.baseline.is_none());
        assert_eq!(app.restored_from.as_ref(), Some(&first));

        let output = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
        assert_eq!(
            title_commands(&output),
            vec![format!("{APP_TITLE} - Recovered")]
        );
        assert_eq!(app.recovery.as_ref().unwrap().path, second);
        app.dismiss_recovery();
        assert!(second.with_extension("resolved").is_file());
        assert!(!first.with_extension("resolved").exists());
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
        assert!(app.recovery.is_none());
        assert_eq!(
            [&first, &second, &original].map(|path| std::fs::read(path).unwrap()),
            snapshots
        );
    }

    #[test]
    fn every_editor_section_renders_in_both_languages_without_network_or_device() {
        for language in ["fr", "en"] {
            let mut app = App {
                startup_checked: true,
                project: Project::named("Synthetic UI").unwrap(),
                ..App::default()
            };
            app.prefs.language = language.into();
            let ctx = egui::Context::default();
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
            app.settings_open = true;
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
            app.selected = true;
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
            assert!(!app.dirty);
            app.settings_open = false;
            for section in [Section::Project, Section::Metadata, Section::Chapters] {
                app.section = section;
                let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
            }
            app.show_export_settings();
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
            assert!(!app.dirty);
            app.export_settings = None;
            app.creating = true;
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.render(ctx));
        }
    }
}
