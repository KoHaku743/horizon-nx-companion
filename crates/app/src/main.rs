//! Horizon NX Companion: the window. Every job runs in `companion-core` on its own thread; this
//! file only lists what's there and draws the jobs' progress.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod theme;

use companion_core::health::{self, Check, Fix, Level, Reading};
use companion_core::registry::{self, Offer, OfferKind, OfferState};
use companion_core::releases::{self, Release};
use companion_core::resources::{self, Age, Plan, Prepared};
use companion_core::saves::{self, Backup, World};
use companion_core::sd::{self, SdCard};
use companion_core::sync::{self, Direction, Plan as SyncPlan};
use companion_core::{Progress, cleanup, logs, size_text};
use eframe::egui::{self, RichText};
use horizon_mod_package::index::Index;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// An empty card reader must not open Windows' "no disk in the drive" box each time the drives are
/// looked through.
#[cfg(windows)]
fn no_disk_boxes() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetErrorMode(mode: u32) -> u32;
    }
    const SEM_FAILCRITICALERRORS: u32 = 1;
    // SAFETY: only sets a flag of this process.
    unsafe { SetErrorMode(SEM_FAILCRITICALERRORS) };
}

fn main() -> eframe::Result {
    #[cfg(windows)]
    no_disk_boxes();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Horizon NX Companion")
            .with_inner_size([940.0, 640.0])
            .with_min_inner_size([720.0, 480.0])
            .with_icon(egui::IconData {
                rgba: include_bytes!("../icon-128.rgba").to_vec(),
                width: 128,
                height: 128,
            }),
        ..Default::default()
    };
    eframe::run_native(
        "Horizon NX Companion",
        options,
        Box::new(|cc| {
            theme::install(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    )
}

/// Something read on a thread (a network list, a folder walk).
struct Loader<T>(Arc<Mutex<Option<Result<T, String>>>>);

impl<T: Send + 'static> Loader<T> {
    fn start(
        ctx: &egui::Context,
        job: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Self {
        let slot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = job();
            *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
            ctx.request_repaint();
        });
        Loader(slot)
    }

    /// Takes the result once it's there.
    fn take(&self) -> Option<Result<T, String>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

/// A value that's loading, loaded or failed.
enum Slot<T> {
    Idle,
    Loading(Loader<T>),
    Ready(T),
    Failed(String),
}

impl<T: Send + 'static> Slot<T> {
    fn poll(&mut self) {
        if let Slot::Loading(loader) = self {
            match loader.take() {
                Some(Ok(v)) => *self = Slot::Ready(v),
                Some(Err(e)) => *self = Slot::Failed(e),
                None => {}
            }
        }
    }

    fn ready(&self) -> Option<&T> {
        match self {
            Slot::Ready(v) => Some(v),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Card,
    GameFiles,
    Build,
    Mods,
    Worlds,
    Sync,
    Logs,
    Cleanup,
}

impl Page {
    const ALL: [(Page, &'static str); 8] = [
        (Page::Card, "Overview"),
        (Page::GameFiles, "Game files"),
        (Page::Build, "Horizon build"),
        (Page::Mods, "Mods and packs"),
        (Page::Worlds, "Worlds"),
        (Page::Sync, "Sync with Horizon"),
        (Page::Logs, "Logs"),
        (Page::Cleanup, "Free up space"),
    ];
}

/// What to read again when a job ends.
#[derive(Clone, Copy, PartialEq, Eq)]
enum After {
    Card,
    /// The card may be gone: look for cards again.
    Cards,
    Mods,
    Worlds,
}

struct Job {
    title: String,
    progress: Progress,
    after: After,
}

struct App {
    page: Page,
    cards: Vec<SdCard>,
    card: Option<usize>,
    job: Option<Job>,
    /// The last job's result.
    notice: Option<Result<String, String>>,

    prepared: Slot<Vec<Prepared>>,
    source: Option<PathBuf>,
    plan: Slot<Plan>,
    plan_for: Option<(PathBuf, PathBuf)>,
    /// The card being read; the pages show the last reading meanwhile.
    reading: Slot<Reading>,
    checks: Vec<Check>,
    source_age: Slot<Age>,
    source_age_for: Option<PathBuf>,
    installed: Option<String>,

    releases: Slot<Vec<Release>>,
    test_builds: bool,

    index: Slot<Arc<Index>>,
    /// Set when the index is a kept copy (the registry is offline).
    index_note: Arc<Mutex<Option<String>>>,
    offers: Vec<Offer>,
    search: String,
    show_kind: Option<OfferKind>,

    switch_worlds: Vec<World>,
    computer_worlds: Vec<World>,
    backups: Vec<Backup>,
    /// The worlds' icons, by folder (None: not a picture).
    icons: HashMap<PathBuf, Option<egui::TextureHandle>>,
    /// The last look for cards in the background, and when it started.
    watch: Slot<Vec<SdCard>>,
    watched: std::time::Instant,
    to_switch: SyncPlan,
    to_computer: SyncPlan,

    log_files: Vec<PathBuf>,
    log: Option<(PathBuf, String)>,
    problems_only: bool,

    cleanables: Vec<(cleanup::Cleanable, bool)>,
    started: bool,
}

impl App {
    fn new() -> Self {
        App {
            page: Page::Card,
            cards: Vec::new(),
            card: None,
            job: None,
            notice: None,
            prepared: Slot::Idle,
            source: None,
            plan: Slot::Idle,
            plan_for: None,
            reading: Slot::Idle,
            checks: Vec::new(),
            source_age: Slot::Idle,
            source_age_for: None,
            installed: None,
            releases: Slot::Idle,
            test_builds: false,
            index: Slot::Idle,
            index_note: Arc::default(),
            offers: Vec::new(),
            search: String::new(),
            show_kind: None,
            switch_worlds: Vec::new(),
            computer_worlds: Vec::new(),
            backups: Vec::new(),
            icons: HashMap::new(),
            watch: Slot::Idle,
            watched: std::time::Instant::now(),
            to_switch: SyncPlan::default(),
            to_computer: SyncPlan::default(),
            log_files: Vec::new(),
            log: None,
            problems_only: false,
            cleanables: Vec::new(),
            started: false,
        }
    }

    fn card(&self) -> Option<&SdCard> {
        self.card.and_then(|i| self.cards.get(i))
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Looks for SD cards again, keeping the chosen one if it's still there.
    fn find_cards(&mut self, ctx: &egui::Context) {
        let found = sd::find();
        self.take_cards(ctx, found);
    }

    /// `found`, and the drives the player chose that are still there.
    fn take_cards(&mut self, ctx: &egui::Context, mut found: Vec<SdCard>) {
        let chosen = self.card().map(|c| c.root.clone());
        let known: Vec<PathBuf> = found.iter().map(|c| c.root.clone()).collect();
        found.extend(
            self.cards
                .iter()
                .filter(|c| !known.contains(&c.root))
                .filter_map(|c| SdCard::at(&c.root)),
        );
        self.cards = found;
        self.card = chosen
            .and_then(|root| self.cards.iter().position(|c| c.root == root))
            .or(if self.cards.is_empty() { None } else { Some(0) });
        self.read_card(ctx);
    }

    /// Starts reading what's on the chosen card, forgetting what another card held.
    fn read_card(&mut self, ctx: &egui::Context) {
        self.plan = Slot::Idle;
        self.plan_for = None;
        self.installed = None;
        self.checks.clear();
        self.switch_worlds.clear();
        self.to_switch = SyncPlan::default();
        self.to_computer = SyncPlan::default();
        self.log_files.clear();
        self.log = None;
        self.cleanables.clear();
        self.reading = match self.card().cloned() {
            Some(card) => Slot::Loading(Loader::start(ctx, move || Ok(health::read(&card)))),
            None => Slot::Idle,
        };
        if self.card().is_some() && matches!(self.releases, Slot::Idle) {
            self.releases = Slot::Loading(Loader::start(ctx, releases::list));
        }
        self.read_offers();
    }

    /// Takes a finished reading over into the pages.
    fn show_reading(&mut self) {
        let Some(r) = self.reading.ready().cloned() else {
            return;
        };
        self.installed = r.installed;
        self.switch_worlds = r.switch_worlds;
        self.computer_worlds = r.computer_worlds;
        self.backups = r.backups;
        self.icons.clear();
        self.to_switch = r.to_switch;
        self.to_computer = r.to_computer;
        self.log_files = r.log_files;
        self.cleanables = r
            .cleanables
            .into_iter()
            .map(|c| {
                let on = c.suggested;
                (c, on)
            })
            .collect();
    }

    /// Notices a card that's plugged in or pulled out (not while a job runs).
    fn watch_cards(&mut self, ctx: &egui::Context) {
        const EVERY: std::time::Duration = std::time::Duration::from_secs(3);
        self.watch.poll();
        if self.busy() {
            // A look from before the job says nothing about after it.
            self.watch = Slot::Idle;
            self.watched = std::time::Instant::now();
            return;
        }
        if matches!(self.watch, Slot::Ready(_) | Slot::Failed(_))
            && let Slot::Ready(found) = std::mem::replace(&mut self.watch, Slot::Idle)
        {
            let gone = self.cards.iter().any(|c| !c.root.is_dir());
            let new = found
                .iter()
                .any(|f| !self.cards.iter().any(|c| c.root == f.root));
            if gone || new {
                self.take_cards(ctx, found);
            }
        } else if matches!(self.watch, Slot::Idle) && self.watched.elapsed() >= EVERY {
            self.watched = std::time::Instant::now();
            self.watch = Slot::Loading(Loader::start(ctx, || Ok(sd::find())));
        }
        ctx.request_repaint_after(EVERY);
    }

    /// The newest full release, once the list is read.
    fn newest(&self) -> Option<&Release> {
        self.releases.ready()?.iter().find(|r| !r.prerelease)
    }

    fn update_checks(&mut self) {
        self.checks = self
            .reading
            .ready()
            .map(|r| health::checks(r, self.newest()))
            .unwrap_or_default();
    }

    /// The age of the chosen prepared files, read once per folder.
    fn poll_source_age(&mut self, ctx: &egui::Context) {
        if self.source != self.source_age_for {
            self.source_age_for = self.source.clone();
            self.source_age = match self.source.clone() {
                Some(source) => {
                    Slot::Loading(Loader::start(ctx, move || Ok(resources::age(&source))))
                }
                None => Slot::Idle,
            };
        }
        self.source_age.poll();
    }

    fn read_offers(&mut self) {
        if let Some(index) = self.index.ready() {
            self.offers = registry::offers(index, self.card());
        }
    }

    fn start(
        &mut self,
        title: impl Into<String>,
        after: After,
        job: impl FnOnce(&Progress) -> Result<String, String> + Send + 'static,
    ) {
        if self.busy() {
            return;
        }
        let progress = Progress::new();
        let title = title.into();
        progress.spawn(&title, job);
        self.notice = None;
        self.job = Some(Job {
            title,
            progress,
            after,
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if !self.started {
            self.started = true;
            self.find_cards(ctx);
            self.prepared = Slot::Loading(Loader::start(ctx, || Ok(resources::find_prepared())));
        }
        self.prepared.poll();
        if self.source.is_none()
            && let Some(found) = self.prepared.ready().and_then(|p| p.first())
        {
            self.source = Some(found.folder.clone());
        }
        self.poll_source_age(ctx);
        self.watch_cards(ctx);
        self.plan.poll();
        let pending = |app: &App| {
            (
                matches!(app.reading, Slot::Loading(_)),
                matches!(app.releases, Slot::Loading(_)),
            )
        };
        let before = pending(self);
        self.reading.poll();
        self.releases.poll();
        let now = pending(self);
        if before.0 && !now.0 {
            self.show_reading();
        }
        if before != now {
            self.update_checks();
        }
        let had_index = self.index.ready().is_some();
        self.index.poll();
        if !had_index && self.index.ready().is_some() {
            self.read_offers();
        }
        let finished = self
            .job
            .as_ref()
            .and_then(|j| j.progress.state().finished.map(|r| (r, j.after)));
        if let Some((result, after)) = finished {
            self.job = None;
            self.notice = Some(result);
            match after {
                After::Card | After::Worlds => self.read_card(ctx),
                After::Cards => self.find_cards(ctx),
                After::Mods => self.read_offers(),
            }
        }
        if self.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        theme::follow(&ctx);

        egui::Panel::top("card").show(ui, |ui| self.card_bar(ui));
        egui::Panel::bottom("job").show(ui, |ui| self.job_bar(ui));
        egui::Panel::left("pages").exact_size(184.0).show(ui, |ui| {
            ui.add_space(10.0);
            for (page, label) in Page::ALL {
                let wrong =
                    page == Page::Card && self.checks.iter().any(|c| c.level == Level::Broken);
                let text = if wrong {
                    bad(label)
                } else {
                    RichText::new(label)
                };
                let button = theme::page_button(text, self.page == page)
                    .min_size(egui::vec2(ui.available_width(), 32.0))
                    .right_text("");
                if ui.add(button).clicked() {
                    self.page = page;
                }
            }
        });
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| match self.page {
                    Page::Card => self.card_page(ui),
                    Page::GameFiles => self.game_files_page(ui),
                    Page::Build => self.build_page(ui),
                    Page::Mods => self.mods_page(ui),
                    Page::Worlds => self.worlds_page(ui),
                    Page::Sync => self.sync_page(ui),
                    Page::Logs => self.logs_page(ui),
                    Page::Cleanup => self.cleanup_page(ui),
                });
        });
    }
}

fn reveal(path: &Path) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(path).spawn();
}

fn days_ago(time: std::time::SystemTime) -> String {
    match time.elapsed().map_or(0, |e| e.as_secs() / 86_400) {
        0 => "played in the last day".into(),
        1 => "played 1 day ago".into(),
        n => format!("played {n} days ago"),
    }
}

fn weak(text: impl Into<String>) -> RichText {
    RichText::new(text).weak()
}

fn good(text: impl Into<String>) -> RichText {
    RichText::new(text).color(theme::colours().success)
}

fn bad(text: impl Into<String>) -> RichText {
    RichText::new(text).color(theme::colours().danger)
}

impl App {
    fn card_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("SD card:");
            let text = self
                .card()
                .map_or_else(|| "None found".to_owned(), |c| c.name.clone());
            egui::ComboBox::from_id_salt("cards")
                .selected_text(text)
                .show_ui(ui, |ui| {
                    let mut chosen = self.card;
                    for (i, card) in self.cards.iter().enumerate() {
                        ui.selectable_value(&mut chosen, Some(i), &card.name);
                    }
                    if chosen != self.card {
                        self.card = chosen;
                        self.read_card(ui.ctx());
                    }
                });
            if ui.add_enabled(!self.busy(), egui::Button::new("Look again")).clicked() {
                self.find_cards(ui.ctx());
            }
            if ui.add_enabled(!self.busy(), egui::Button::new("Choose a drive…")).clicked()
                && let Some(root) = rfd::FileDialog::new().pick_folder() {
                    match SdCard::at(&root) {
                        Some(card) => {
                            self.cards.retain(|c| c.root != card.root);
                            self.cards.push(card);
                            self.card = Some(self.cards.len() - 1);
                            self.read_card(ui.ctx());
                        }
                        None => {
                            self.notice = Some(Err(format!(
                                "{} doesn't look like a Switch SD card (no atmosphere or switch folder).",
                                root.display()
                            )))
                        }
                    }
                }
            if let Some(card) = self.card().cloned() {
                if let Some((free, total)) = card.space() {
                    ui.label(weak(format!("{} free of {}", size_text(free), size_text(total))));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(!self.busy(), egui::Button::new("Eject")).clicked() {
                        let card = card.clone();
                        self.start("Ejecting", After::Cards, move |p| {
                            p.step("Removing macOS's hidden files", 0);
                            sd::remove_mac_junk(&card.horizon());
                            p.step("Ejecting", 0);
                            sd::eject(&card)?;
                            Ok(format!("{} is ejected: you can unplug it.", card.name))
                        });
                    }
                    if ui.button("Open").clicked() {
                        reveal(&card.horizon());
                    }
                });
            }
        });
        ui.add_space(4.0);
    }

    fn job_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        if let Some(job) = &self.job {
            let state = job.progress.state();
            ui.horizontal(|ui| {
                ui.label(RichText::new(&job.title).strong());
                ui.label(&state.step);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Stop").clicked() {
                        job.progress.cancel();
                    }
                });
            });
            let bar = if state.total > 0 {
                let f = state.done as f32 / state.total as f32;
                egui::ProgressBar::new(f.min(1.0)).text(format!(
                    "{} of {}",
                    size_text(state.done),
                    size_text(state.total)
                ))
            } else {
                egui::ProgressBar::new(0.0).animate(true)
            };
            ui.add(bar);
        } else {
            match &self.notice {
                Some(Ok(text)) => {
                    ui.label(good(text));
                }
                Some(Err(text)) => {
                    ui.label(bad(text));
                }
                None => {
                    ui.label(weak("Ready."));
                }
            }
        }
        ui.add_space(4.0);
    }

    fn no_card(&self, ui: &mut egui::Ui) -> Option<SdCard> {
        let card = self.card().cloned();
        if card.is_none() {
            ui.label("Put the Switch's SD card in this computer (or start hekate's USB tools: Tools > USB Tools > SD Card). It shows up here by itself.");
        }
        card
    }

    fn card_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Overview");
        ui.add_space(6.0);
        let Some(card) = self.no_card(ui) else {
            return;
        };
        if matches!(self.reading, Slot::Loading(_)) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading the card…");
            });
            return;
        }
        if let Slot::Failed(e) = &self.reading {
            ui.label(bad(e));
            return;
        }
        let wrong = |c: &&Check| c.level != Level::Good;
        let mut fixes: Vec<Fix> = self
            .checks
            .iter()
            .filter(wrong)
            .filter_map(|c| c.fix)
            .filter(|f| self.can_fix(*f))
            .collect();
        fixes.sort();
        if self.checks.iter().all(|c| c.level == Level::Good) {
            ui.label(good("Ready to play."));
        } else if fixes.len() > 1 {
            let label = format!("Fix all {}", fixes.len());
            if ui
                .add_enabled(!self.busy(), theme::main_button(label))
                .clicked()
            {
                self.fix(fixes, &card);
            }
        }
        ui.add_space(8.0);
        for check in self.checks.clone() {
            let colours = theme::colours();
            let colour = match check.level {
                Level::Good => colours.success,
                Level::Warning => colours.warn,
                Level::Broken => colours.danger,
            };
            ui.separator();
            ui.horizontal_top(|ui| {
                ui.label(RichText::new("●").color(colour));
                let button = check.fix.map(|_| 170.0).unwrap_or(0.0);
                let width = (ui.available_width() - button).max(200.0);
                ui.allocate_ui(egui::vec2(width, 0.0), |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        ui.label(RichText::new(&check.title).strong());
                        ui.label(weak(&check.detail));
                        if check.fix == Some(Fix::GameFiles) && self.source.is_none() {
                            ui.label(weak("No prepared files found on this computer: start the Horizon launcher and press Play once, or choose them on the Game files page."));
                        }
                    });
                });
                let Some(fix) = check.fix else {
                    return;
                };
                let label = match fix {
                    Fix::InstallBuild => "Install the newest",
                    Fix::ClientTxt => "Put it back",
                    Fix::GameFiles => "Copy and pack",
                    Fix::HiddenNames => "Rename them",
                    Fix::MacJunk => "Remove them",
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    let able = !self.busy() && self.can_fix(fix);
                    if ui.add_enabled(able, egui::Button::new(label)).clicked() {
                        self.fix(vec![fix], &card);
                    }
                });
            });
        }
        ui.separator();
        ui.add_space(8.0);
        ui.label(weak(format!(
            "{} · {} worlds",
            card.root.display(),
            self.switch_worlds.len()
        )));
        ui.label(weak(
            "Eject the card before you unplug it (or before you leave hekate's USB mode), so nothing is lost.",
        ));
    }

    /// Whether what `fix` needs is there.
    fn can_fix(&self, fix: Fix) -> bool {
        match fix {
            Fix::InstallBuild => self.newest().is_some(),
            Fix::GameFiles => self.source.is_some(),
            _ => true,
        }
    }

    fn fix(&mut self, fixes: Vec<Fix>, card: &SdCard) {
        let (card, newest, source) = (card.clone(), self.newest().cloned(), self.source.clone());
        self.start("Fixing the card", After::Card, move |p| {
            health::apply(&fixes, &card, newest.as_ref(), source.as_deref(), p)
        });
    }

    fn game_files_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Game files");
        ui.label(weak(
            "Horizon needs Minecraft 26.3's files, prepared by the Horizon launcher on this computer from your own Minecraft. This copies them to the card and packs them into resources.hzpak, the way the Switch would on its first start (which takes it many minutes).",
        ));
        ui.add_space(8.0);
        let Some(card) = self.no_card(ui) else {
            return;
        };
        ui.horizontal(|ui| {
            ui.label("Prepared files:");
            let text = self
                .source
                .as_ref()
                .map_or_else(|| "none found".to_owned(), |p| p.display().to_string());
            egui::ComboBox::from_id_salt("prepared")
                .width(420.0)
                .selected_text(text)
                .show_ui(ui, |ui| {
                    if let Some(list) = self.prepared.ready() {
                        for found in list {
                            ui.selectable_value(&mut self.source, Some(found.folder.clone()), found.folder.display().to_string());
                        }
                    }
                });
            if ui.button("Choose…").clicked()
                && let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    if let Some(found) = resources::find_in(&folder) {
                        self.source = Some(found);
                    } else {
                        self.notice = Some(Err(format!(
                            "No prepared files in {} (no {}): pick the Horizon launcher's folder, Library/Application Support/horizon-launcher.",
                            folder.display(),
                            resources::MANIFEST
                        )));
                    }
                }
        });
        if matches!(self.prepared, Slot::Loading(_)) {
            ui.label(weak("Looking for prepared files…"));
        } else if self.source.is_none() {
            ui.label("None found. Start the Horizon launcher once on this computer (it prepares the files from your Minecraft), or choose the folder.");
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        let age = match self.source_age.ready() {
            Some(Age::TooOld) => Some(
                "These files are from before Horizon 2.0: the game would close after loading. Update the Horizon launcher and press Play once, then choose the new ones.",
            ),
            Some(Age::OtherMinecraft) => {
                Some("These files aren't Minecraft 26.3's: Horizon won't start with them.")
            }
            Some(Age::Damaged) => {
                Some("These files are incomplete: prepare them again with the Horizon launcher.")
            }
            _ => None,
        };
        if let Some(why) = age {
            ui.label(bad(why));
        }
        let key = (source.clone(), card.root.clone());
        if self.plan_for.as_ref() != Some(&key) && !self.busy() {
            self.plan_for = Some(key);
            let (s, c) = (source.clone(), card.clone());
            self.plan = Slot::Loading(Loader::start(ui.ctx(), move || {
                resources::plan(&s, &c, &Progress::new())
            }));
        }
        ui.add_space(8.0);
        match &self.plan {
            Slot::Ready(plan) => {
                ui.label(format!(
                    "{} files, {}; {} of them are on the card already.",
                    plan.files,
                    size_text(plan.bytes),
                    size_text(plan.unchanged_bytes)
                ));
                ui.label(format!("The pack: {}.", size_text(plan.pack_bytes)));
                let room = plan.free.is_none_or(|free| free >= plan.needed);
                if !room {
                    ui.label(bad(format!(
                        "The card needs {} free for this.",
                        size_text(plan.needed)
                    )));
                }
                ui.add_space(6.0);
                if ui
                    .add_enabled(room && !self.busy(), theme::main_button("Copy and pack"))
                    .clicked()
                {
                    self.start("Game files", After::Card, move |p| {
                        resources::copy_and_pack(&source, &card, p)
                    });
                }
            }
            Slot::Failed(e) => {
                ui.label(bad(e));
            }
            _ => {
                ui.label(weak("Measuring…"));
            }
        }
    }

    fn build_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Horizon build");
        ui.label(weak(format!(
            "horizon.nro from {}'s releases, checked and put in switch/horizon/.",
            releases::REPOSITORY
        )));
        ui.add_space(8.0);
        let Some(card) = self.no_card(ui) else {
            return;
        };
        ui.label(format!(
            "On the card: {}",
            self.installed.clone().unwrap_or_else(|| "nothing".into())
        ));
        ui.horizontal(|ui| {
            if ui.button("Check for builds").clicked() || matches!(self.releases, Slot::Idle) {
                self.releases = Slot::Loading(Loader::start(ui.ctx(), releases::list));
            }
            ui.checkbox(&mut self.test_builds, "Show test builds");
        });
        ui.add_space(6.0);
        match &self.releases {
            Slot::Ready(list) => {
                let list: Vec<Release> = list
                    .iter()
                    .filter(|r| self.test_builds || !r.prerelease)
                    .cloned()
                    .collect();
                if list.is_empty() {
                    ui.label("No builds.");
                }
                for (i, release) in list.into_iter().enumerate() {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(
                                    release
                                        .name
                                        .clone()
                                        .unwrap_or_else(|| release.tag_name.clone()),
                                )
                                .strong(),
                            );
                            if release.prerelease {
                                ui.label(weak("test build"));
                            } else if i == 0 {
                                ui.label(good("newest"));
                            }
                            ui.label(weak(release.date()));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let here = self.installed.as_deref() == Some(release.version());
                                    let label = if here { "Install again" } else { "Install" };
                                    if ui
                                        .add_enabled(!self.busy(), egui::Button::new(label))
                                        .clicked()
                                    {
                                        let card = card.clone();
                                        let r = release.clone();
                                        self.start(
                                            format!("Installing {}", release.tag_name),
                                            After::Card,
                                            move |p| releases::install(&r, &card, p),
                                        );
                                    }
                                    if ui.button("Notes").clicked() {
                                        reveal(Path::new(&release.html_url));
                                    }
                                },
                            );
                        });
                    });
                }
            }
            Slot::Failed(e) => {
                ui.label(bad(e));
            }
            _ => {
                ui.label(weak("Reading the releases…"));
            }
        }
        ui.add_space(10.0);
        ui.label(weak(
            "Building horizon.nro on this computer (for native mods) comes later.",
        ));
    }

    fn mods_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Mods and packs");
        ui.label(weak(
            "Content mods and resource packs from Horizon's registry, checked here exactly as the game's Get Mods checks them, then put on the card and turned on.",
        ));
        ui.add_space(8.0);
        let card = self.no_card(ui);
        if matches!(self.index, Slot::Idle) {
            let (card, note) = (card.clone(), self.index_note.clone());
            self.index = Slot::Loading(Loader::start(ui.ctx(), move || {
                let (index, kept) = registry::load_index_or_kept(card.as_ref())?;
                *note.lock().unwrap_or_else(|e| e.into_inner()) = kept;
                Ok(Arc::new(index))
            }));
        }
        let index = match &self.index {
            Slot::Ready(index) => index.clone(),
            Slot::Failed(e) => {
                ui.label(bad(e));
                if ui.button("Try again").clicked() {
                    self.index = Slot::Idle;
                }
                return;
            }
            _ => {
                ui.label(weak("Reading the registry…"));
                return;
            }
        };
        let note = self
            .index_note
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(note) = note {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(note).color(theme::colours().warn));
                if ui.small_button("Try again").clicked() {
                    self.index = Slot::Idle;
                }
            });
            ui.add_space(6.0);
        }
        ui.horizontal(|ui| {
            ui.label("Search:");
            ui.text_edit_singleline(&mut self.search);
            ui.selectable_value(&mut self.show_kind, None, "All");
            ui.selectable_value(&mut self.show_kind, Some(OfferKind::Mod), "Mods");
            ui.selectable_value(&mut self.show_kind, Some(OfferKind::ResourcePack), "Resource packs");
            let unchecked = self.offers.iter().filter(|o| o.state == OfferState::Unchecked).count();
            if unchecked > 0
                && ui
                    .add_enabled(!self.busy(), egui::Button::new(format!("Check all {unchecked} mods")))
                    .on_hover_text("Downloads and checks every mod not checked yet, so the list says which ones work on the Switch build.")
                    .clicked()
            {
                let index = index.clone();
                self.start("Checking mods", After::Mods, move |p| registry::check_all(&index, p));
            }
        });
        ui.add_space(6.0);
        let needle = self.search.to_lowercase();
        let shown: Vec<Offer> = self
            .offers
            .iter()
            .filter(|o| self.show_kind.is_none_or(|k| o.kind == k))
            .filter(|o| {
                needle.is_empty()
                    || o.name.to_lowercase().contains(&needle)
                    || o.summary.to_lowercase().contains(&needle)
            })
            .cloned()
            .collect();
        if shown.is_empty() {
            ui.label("Nothing matches.");
        }
        for offer in shown {
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&offer.name).strong());
                            ui.label(weak(format!(
                                "{} · {} · {}",
                                offer.version,
                                size_text(offer.size),
                                if offer.kind == OfferKind::Mod {
                                    "mod"
                                } else {
                                    "resource pack"
                                }
                            )));
                        });
                        if !offer.summary.is_empty() {
                            ui.label(&offer.summary);
                        }
                        match &offer.state {
                            OfferState::Installed => {
                                ui.label(good("On the card"));
                            }
                            OfferState::Update => {
                                ui.label(good("On the card, an older version"));
                            }
                            OfferState::Available if offer.kind == OfferKind::Mod => {
                                ui.label(good("Works on the Switch build"));
                            }
                            OfferState::Unsupported(why)
                                if why == companion_core::registry::CODE_MOD =>
                            {
                                ui.label(bad("Runs code: not for the Switch"))
                                    .on_hover_text(why);
                            }
                            OfferState::Unsupported(why) => {
                                ui.label(bad("Needs a newer Horizon")).on_hover_text(why);
                            }
                            _ => {}
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let Some(card) = card.clone() else {
                            return;
                        };
                        let free = !self.busy();
                        match offer.state {
                            OfferState::Installed | OfferState::Update => {
                                if ui.add_enabled(free, egui::Button::new("Remove")).clicked() {
                                    self.notice = Some(registry::remove(&index, &offer.id, &card));
                                    self.read_offers();
                                }
                                if offer.state == OfferState::Update
                                    && ui.add_enabled(free, egui::Button::new("Update")).clicked()
                                {
                                    let (index, id) = (index.clone(), offer.id.clone());
                                    self.start(
                                        format!("Updating {}", offer.name),
                                        After::Mods,
                                        move |p| registry::install(&index, &id, &card, p),
                                    );
                                }
                            }
                            OfferState::Unsupported(_) => {}
                            _ => {
                                if ui.add_enabled(free, egui::Button::new("Install")).clicked() {
                                    let (index, id) = (index.clone(), offer.id.clone());
                                    self.start(
                                        format!("Installing {}", offer.name),
                                        After::Mods,
                                        move |p| registry::install(&index, &id, &card, p),
                                    );
                                }
                            }
                        }
                    });
                });
            });
        }
    }

    fn worlds_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Worlds");
        ui.label(weak(
            "Copy worlds between Horizon on the Switch and Horizon on this computer, and back them up. Nothing is ever replaced: a world whose name is taken gets a number.",
        ));
        ui.add_space(8.0);
        let Some(card) = self.no_card(ui) else {
            return;
        };
        if matches!(self.reading, Slot::Loading(_)) {
            ui.spinner();
            return;
        }
        let computer = saves::computer_saves();
        let switch = saves::switch_saves(&card);

        ui.horizontal(|ui| {
            ui.label(RichText::new("On the Switch").strong());
            if !self.switch_worlds.is_empty()
                && ui
                    .add_enabled(!self.busy(), egui::Button::new("Back up all"))
                    .clicked()
            {
                let worlds = self.switch_worlds.clone();
                self.start("Backing up", After::Worlds, move |p| {
                    saves::backup(&worlds, p)
                });
            }
        });
        if self.switch_worlds.is_empty() {
            ui.label(weak("No worlds."));
        }
        for world in self.switch_worlds.clone() {
            self.world_row(ui, &world, |app, ui| {
                if let Some(to) = computer.clone()
                    && ui
                        .add_enabled(!app.busy(), egui::Button::new("Copy to computer"))
                        .clicked()
                {
                    let world = world.clone();
                    app.start(
                        format!("Copying {}", world.title),
                        After::Worlds,
                        move |p| saves::copy_world(&world, &to, Direction::ToComputer, p),
                    );
                }
                if ui
                    .add_enabled(!app.busy(), egui::Button::new("Back up"))
                    .clicked()
                {
                    let one = [world.clone()];
                    app.start(
                        format!("Backing up {}", world.title),
                        After::Worlds,
                        move |p| saves::backup(&one, p),
                    );
                }
            });
        }

        ui.add_space(16.0);
        ui.label(RichText::new("On this computer (Horizon)").strong());
        if self.computer_worlds.is_empty() {
            ui.label(weak("No worlds."));
        }
        for world in self.computer_worlds.clone() {
            self.world_row(ui, &world, |app, ui| {
                if ui
                    .add_enabled(!app.busy(), egui::Button::new("Copy to Switch"))
                    .clicked()
                {
                    let (world, to) = (world.clone(), switch.clone());
                    app.start(
                        format!("Copying {}", world.title),
                        After::Worlds,
                        move |p| saves::copy_world(&world, &to, Direction::ToSwitch, p),
                    );
                }
            });
        }

        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Backups").strong());
            if ui.small_button("Open folder").clicked() {
                let dir = saves::backup_dir();
                let _ = std::fs::create_dir_all(&dir);
                reveal(&dir);
            }
        });
        if self.backups.is_empty() {
            ui.label(weak("No backups yet."));
        }
        for backup in self.backups.clone() {
            let name = backup.path.file_stem().unwrap_or_default();
            egui::CollapsingHeader::new(format!(
                "{} · {}",
                name.to_string_lossy(),
                size_text(backup.bytes)
            ))
            .id_salt(&backup.path)
            .show(ui, |ui| {
                for world in &backup.worlds {
                    ui.horizontal(|ui| {
                        ui.label(world);
                        let sides = [
                            (
                                "Restore to Switch",
                                Some(switch.clone()),
                                Direction::ToSwitch,
                            ),
                            (
                                "Restore to computer",
                                computer.clone(),
                                Direction::ToComputer,
                            ),
                        ];
                        for (label, to, direction) in sides {
                            let Some(to) = to else {
                                continue;
                            };
                            if ui
                                .add_enabled(!self.busy(), egui::Button::new(label))
                                .clicked()
                            {
                                let (path, world) = (backup.path.clone(), world.clone());
                                self.start(format!("Restoring {world}"), After::Worlds, move |p| {
                                    saves::restore(&path, &world, &to, direction, p)
                                });
                            }
                        }
                    });
                }
            });
        }
    }

    /// A world's icon, names and details, with `buttons` at the right.
    fn world_row(
        &mut self,
        ui: &mut egui::Ui,
        world: &World,
        buttons: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        ui.separator();
        ui.horizontal(|ui| {
            let size = egui::vec2(40.0, 40.0);
            let icon = self
                .icons
                .entry(world.folder.clone())
                .or_insert_with(|| theme::picture(ui.ctx(), world.icon.as_deref()?));
            match icon {
                Some(texture) => {
                    ui.image((texture.id(), size));
                }
                None => {
                    ui.allocate_space(size);
                }
            }
            ui.vertical(|ui| {
                ui.label(RichText::new(&world.title).strong());
                let mut details = vec![size_text(world.bytes)];
                if !world.kind.is_empty() {
                    details.insert(0, world.kind.clone());
                }
                if let Some(played) = world.modified {
                    details.push(days_ago(played));
                }
                if world.name != world.title {
                    details.push(world.name.clone());
                }
                ui.label(weak(details.join(" · ")));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                buttons(self, ui)
            });
        });
    }

    fn sync_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Sync with Horizon");
        ui.label(weak(
            "Mods, resource packs and servers between Horizon on this computer and Horizon on the Switch. A copy adds what the other side lacks and updates what differs; nothing is deleted. Worlds are on the Worlds page.",
        ));
        ui.add_space(8.0);
        let Some(card) = self.no_card(ui) else {
            return;
        };
        let Some(home) = saves::computer_home() else {
            ui.label(bad("This computer has no data folder for Horizon."));
            return;
        };
        if !home.is_dir() {
            ui.label(weak(format!(
                "Horizon hasn't run on this computer yet ({} doesn't exist): only Switch → computer has anything to copy.",
                home.display()
            )));
        } else {
            ui.horizontal(|ui| {
                ui.label(weak(format!(
                    "Horizon on this computer: {}",
                    home.display()
                )));
                if ui.small_button("Open").clicked() {
                    reveal(&home);
                }
            });
        }
        ui.add_space(6.0);
        let sides = [
            (
                "This computer → Switch",
                "Copy to the Switch",
                Direction::ToSwitch,
                self.to_switch.clone(),
                card.horizon(),
            ),
            (
                "Switch → this computer",
                "Copy to this computer",
                Direction::ToComputer,
                self.to_computer.clone(),
                home,
            ),
        ];
        for (title, button, direction, plan, to) in sides {
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title).strong());
                    ui.label(plan.summary());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if plan.is_empty() {
                            button.to_owned()
                        } else {
                            format!("{button} ({})", size_text(plan.bytes()))
                        };
                        if ui
                            .add_enabled(!plan.is_empty() && !self.busy(), egui::Button::new(label))
                            .clicked()
                        {
                            let plan = plan.clone();
                            self.start(button, After::Card, move |p| {
                                sync::apply(&plan, &to, direction, p)
                            });
                        }
                    });
                });
                if !plan.is_empty() || !plan.skipped.is_empty() {
                    egui::CollapsingHeader::new("Details")
                        .id_salt(title)
                        .show(ui, |ui| {
                            for copy in &plan.copies {
                                let what = match copy.what {
                                    sync::What::Mod => "mod",
                                    sync::What::Pack => "resource pack",
                                };
                                let how = if copy.update { "update" } else { "new" };
                                ui.label(format!(
                                    "{} · {what}, {how}, {}",
                                    copy.name,
                                    size_text(copy.bytes)
                                ));
                            }
                            for server in &plan.servers {
                                ui.label(format!("{} · server", sync::server_name(server)));
                            }
                            for (name, why) in &plan.skipped {
                                ui.label(weak(format!("{name} stays: {why}")));
                            }
                        });
                }
            });
            ui.add_space(4.0);
        }
    }

    fn logs_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Logs");
        ui.label(weak(
            "What the game wrote while it ran: the place to look when something goes wrong.",
        ));
        ui.add_space(8.0);
        let Some(card) = self.no_card(ui) else {
            return;
        };
        ui.horizontal(|ui| {
            for file in self.log_files.clone().into_iter().take(6) {
                let name = file
                    .strip_prefix(card.horizon())
                    .map_or_else(|_| file.display().to_string(), |p| p.display().to_string());
                let open = self.log.as_ref().is_some_and(|(p, _)| *p == file);
                if ui.selectable_label(open, name).clicked() {
                    self.log = Some((file.clone(), logs::read(&file, 5000).unwrap_or_else(|e| e)));
                }
            }
            if self.log_files.is_empty() {
                ui.label(weak("No logs on the card."));
            }
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.problems_only, "Only errors and warnings");
            if !self.log_files.is_empty() && ui.button("Save logs for a bug report").clicked() {
                self.notice = Some(logs::zip_for_report(&card).map(|path| {
                    reveal(path.parent().unwrap_or(&path));
                    format!("Saved {}.", path.display())
                }));
            }
        });
        if self.log.is_none()
            && let Some(first) = self.log_files.first().cloned()
        {
            self.log = Some((
                first.clone(),
                logs::read(&first, 5000).unwrap_or_else(|e| e),
            ));
        }
        if let Some((_, text)) = &self.log {
            let shown = if self.problems_only {
                logs::problems(text).join("\n")
            } else {
                text.clone()
            };
            ui.add(
                egui::TextEdit::multiline(&mut shown.as_str())
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(28),
            );
        }
    }

    fn cleanup_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Free up space");
        ui.label(weak(
            "Things the game makes again when it needs them. Worlds, settings, your account, mods and the game files are never listed.",
        ));
        ui.add_space(8.0);
        if self.no_card(ui).is_none() {
            return;
        }
        if self.cleanables.is_empty() {
            ui.label("Nothing to clean up.");
            return;
        }
        for (item, on) in &mut self.cleanables {
            ui.horizontal(|ui| {
                ui.checkbox(on, format!("{} — {}", item.label, size_text(item.bytes)));
            });
            ui.label(weak(format!("      {}", item.note)));
        }
        let chosen: Vec<cleanup::Cleanable> = self
            .cleanables
            .iter()
            .filter(|(_, on)| *on)
            .map(|(c, _)| c.clone())
            .collect();
        let bytes: u64 = chosen.iter().map(|c| c.bytes).sum();
        ui.add_space(8.0);
        if ui
            .add_enabled(
                !chosen.is_empty() && !self.busy(),
                egui::Button::new(format!("Remove ({})", size_text(bytes))),
            )
            .clicked()
        {
            self.start("Cleaning up", After::Card, move |_| cleanup::clean(&chosen));
        }
    }
}
