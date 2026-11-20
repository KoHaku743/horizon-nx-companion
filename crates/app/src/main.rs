//! Horizon NX Companion: the window. Every job runs in `companion-core` on its own thread; this
//! file only lists what's there and draws the jobs' progress.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use companion_core::registry::{self, Offer, OfferKind, OfferState};
use companion_core::releases::{self, Release};
use companion_core::resources::{self, OnCard, Plan, Prepared};
use companion_core::saves::{self, World};
use companion_core::sd::{self, SdCard};
use companion_core::{Progress, cleanup, logs, size_text};
use eframe::egui::{self, Color32, RichText};
use horizon_mods::index::Index;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Horizon NX Companion")
            .with_inner_size([940.0, 640.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Horizon NX Companion",
        options,
        Box::new(|_| Ok(Box::new(App::new()))),
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
    Logs,
    Cleanup,
}

impl Page {
    const ALL: [(Page, &'static str); 7] = [
        (Page::Card, "SD card"),
        (Page::GameFiles, "Game files"),
        (Page::Build, "Horizon build"),
        (Page::Mods, "Mods and packs"),
        (Page::Worlds, "Worlds"),
        (Page::Logs, "Logs"),
        (Page::Cleanup, "Free up space"),
    ];
}

/// What to read again when a job ends.
#[derive(Clone, Copy, PartialEq, Eq)]
enum After {
    Card,
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
    on_card: Option<OnCard>,
    installed: Option<String>,

    releases: Slot<Vec<Release>>,
    test_builds: bool,

    index: Slot<Arc<Index>>,
    offers: Vec<Offer>,
    search: String,
    show_kind: Option<OfferKind>,

    switch_worlds: Vec<World>,
    computer_worlds: Vec<World>,

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
            on_card: None,
            installed: None,
            releases: Slot::Idle,
            test_builds: false,
            index: Slot::Idle,
            offers: Vec::new(),
            search: String::new(),
            show_kind: None,
            switch_worlds: Vec::new(),
            computer_worlds: Vec::new(),
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
    fn find_cards(&mut self) {
        let chosen = self.card().map(|c| c.root.clone());
        self.cards = sd::find();
        self.card = chosen
            .and_then(|root| self.cards.iter().position(|c| c.root == root))
            .or(if self.cards.is_empty() { None } else { Some(0) });
        self.read_card();
    }

    /// Reads what's on the chosen card (quick things only).
    fn read_card(&mut self) {
        let card = self.card().cloned();
        self.on_card = card.as_ref().map(resources::on_card);
        self.installed = card.as_ref().and_then(releases::installed_version);
        self.switch_worlds = card
            .as_ref()
            .map(|c| saves::worlds_in(&saves::switch_saves(c)))
            .unwrap_or_default();
        self.computer_worlds = saves::computer_saves()
            .map(|s| saves::worlds_in(&s))
            .unwrap_or_default();
        self.log_files = card.as_ref().map(logs::files).unwrap_or_default();
        self.cleanables = card
            .as_ref()
            .map(cleanup::scan)
            .unwrap_or_default()
            .into_iter()
            .map(|c| {
                let on = c.suggested;
                (c, on)
            })
            .collect();
        self.plan = Slot::Idle;
        self.plan_for = None;
        self.read_offers();
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
            self.find_cards();
            self.prepared = Slot::Loading(Loader::start(ctx, || Ok(resources::find_prepared())));
        }
        self.prepared.poll();
        if self.source.is_none()
            && let Some(found) = self.prepared.ready().and_then(|p| p.first())
        {
            self.source = Some(found.folder.clone());
        }
        self.plan.poll();
        self.releases.poll();
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
                After::Card => self.read_card(),
                After::Mods => self.read_offers(),
                After::Worlds => self.read_card(),
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

        egui::Panel::top("card").show(ui, |ui| self.card_bar(ui));
        egui::Panel::bottom("job").show(ui, |ui| self.job_bar(ui));
        egui::Panel::left("pages").exact_size(170.0).show(ui, |ui| {
            ui.add_space(8.0);
            for (page, label) in Page::ALL {
                if ui
                    .add_sized(
                        [150.0, 28.0],
                        egui::Button::selectable(self.page == page, label),
                    )
                    .clicked()
                {
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

fn weak(text: impl Into<String>) -> RichText {
    RichText::new(text).weak()
}

fn good(text: impl Into<String>) -> RichText {
    RichText::new(text).color(Color32::from_rgb(90, 180, 90))
}

fn bad(text: impl Into<String>) -> RichText {
    RichText::new(text).color(Color32::from_rgb(220, 110, 90))
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
                        self.read_card();
                    }
                });
            if ui.add_enabled(!self.busy(), egui::Button::new("Look again")).clicked() {
                self.find_cards();
            }
            if ui.add_enabled(!self.busy(), egui::Button::new("Choose a drive…")).clicked()
                && let Some(root) = rfd::FileDialog::new().pick_folder() {
                    match SdCard::at(&root) {
                        Some(card) => {
                            self.cards.retain(|c| c.root != card.root);
                            self.cards.push(card);
                            self.card = Some(self.cards.len() - 1);
                            self.read_card();
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
                        sd::remove_mac_junk(&card.horizon());
                        self.notice = Some(sd::eject(&card).map(|_| {
                            format!("{} is ejected: you can unplug it.", card.name)
                        }));
                        self.find_cards();
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
            ui.label("Put the Switch's SD card in this computer (or start hekate's USB tools: Tools > USB Tools > SD Card), then press Look again.");
        }
        card
    }

    fn card_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("SD card");
        ui.add_space(6.0);
        let Some(card) = self.no_card(ui) else {
            return;
        };
        egui::Grid::new("card")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.label("Drive");
                ui.label(card.root.display().to_string());
                ui.end_row();
                ui.label("Horizon");
                if card.has_horizon {
                    ui.label(good("switch/horizon is there"));
                } else {
                    ui.label(bad("not on this card yet: install a build first"));
                }
                ui.end_row();
                ui.label("Build");
                ui.label(
                    self.installed
                        .clone()
                        .map_or_else(|| weak("none"), RichText::new),
                );
                ui.end_row();
                ui.label("Game files");
                ui.label(match self.on_card {
                    Some(OnCard::Packed) => good("copied and packed: the game starts fast"),
                    Some(OnCard::Unpacked) => {
                        bad("copied, not packed: the first start will be slow")
                    }
                    Some(OnCard::Stale) => bad("the pack is out of date: pack them again"),
                    _ => bad("not there: copy them from Game files"),
                });
                ui.end_row();
                ui.label("Worlds");
                ui.label(format!("{}", self.switch_worlds.len()));
                ui.end_row();
            });
        ui.add_space(12.0);
        ui.label(weak(
            "Eject the card before you unplug it (or before you leave hekate's USB mode), so nothing is lost.",
        ));
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
                    .add_enabled(
                        room && !self.busy(),
                        egui::Button::new(RichText::new("Copy and pack").strong()),
                    )
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
            self.index = Slot::Loading(Loader::start(ui.ctx(), || {
                registry::load_index().map(Arc::new)
            }));
        }
        let index = match &self.index {
            Slot::Ready(index) => index.clone(),
            Slot::Failed(e) => {
                ui.label(bad(format!("Couldn't read the registry: {e}")));
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
            "Horizon plays Java Edition worlds: copy them between the Switch and Minecraft on this computer. A world from an older Minecraft is upgraded when it's opened; one from a newer Minecraft than 26.3 may not open.",
        ));
        ui.add_space(8.0);
        let Some(card) = self.no_card(ui) else {
            return;
        };
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.busy() && !self.switch_worlds.is_empty(),
                    egui::Button::new("Back up the Switch's worlds"),
                )
                .clicked()
            {
                let card = card.clone();
                self.start("Backing up", After::Worlds, move |p| {
                    saves::backup(&card, p)
                });
            }
            if ui.button("Open backups").clicked() {
                let dir = saves::backup_dir();
                let _ = std::fs::create_dir_all(&dir);
                reveal(&dir);
            }
        });
        ui.add_space(8.0);
        let computer = saves::computer_saves();
        ui.columns(2, |cols| {
            cols[0].label(RichText::new("On the Switch").strong());
            if self.switch_worlds.is_empty() {
                cols[0].label(weak("No worlds."));
            }
            for world in self.switch_worlds.clone() {
                cols[0].horizontal(|ui| {
                    ui.label(format!("{} ({})", world.name, size_text(world.bytes)));
                    if let Some(to) = computer.clone()
                        && ui
                            .add_enabled(!self.busy(), egui::Button::new("To computer →"))
                            .clicked()
                    {
                        self.start(format!("Copying {}", world.name), After::Worlds, move |p| {
                            saves::copy_world(&world, &to, p)
                        });
                    }
                });
            }
            cols[1].label(RichText::new("On this computer (Minecraft)").strong());
            if self.computer_worlds.is_empty() {
                cols[1].label(weak("No worlds."));
            }
            for world in self.computer_worlds.clone() {
                cols[1].horizontal(|ui| {
                    let to = saves::switch_saves(&card);
                    if ui
                        .add_enabled(!self.busy(), egui::Button::new("← To Switch"))
                        .clicked()
                    {
                        let world = world.clone();
                        self.start(format!("Copying {}", world.name), After::Worlds, move |p| {
                            saves::copy_world(&world, &to, p)
                        });
                    }
                    ui.label(format!("{} ({})", world.name, size_text(world.bytes)));
                });
            }
        });
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
