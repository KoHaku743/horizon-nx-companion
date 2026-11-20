//! What Horizon on the Switch would otherwise do itself, done on a computer, and the SD card's
//! upkeep: the game files and their pack, builds, mods and resource packs, saves, logs.
//!
//! Every job runs on its own thread and reports through a [`Progress`]; the app only draws it.

pub mod cleanup;
pub mod hzpak;
pub mod logs;
pub mod nbt;
pub mod registry;
pub mod releases;
pub mod resources;
pub mod saves;
pub mod sd;
pub mod sync;

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

/// A job's state, shared between its thread and the window.
#[derive(Clone, Default)]
pub struct Progress(Arc<Shared>);

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    cancel: AtomicBool,
}

/// What the window shows of a job.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// What it is doing now ("Copying the game files").
    pub step: String,
    pub done: u64,
    /// 0 while unknown.
    pub total: u64,
    /// Set when the job ended: `Ok(summary)` or `Err(why)`.
    pub finished: Option<Result<String, String>>,
}

impl Progress {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> State {
        self.0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn step(&self, step: impl Into<String>, total: u64) {
        let mut s = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        s.step = step.into();
        s.done = 0;
        s.total = total;
    }

    pub fn advance(&self, by: u64) {
        let mut s = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        s.done += by;
    }

    pub fn set_total(&self, total: u64) {
        self.0.state.lock().unwrap_or_else(|e| e.into_inner()).total = total;
    }

    pub fn finish(&self, result: Result<String, String>) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .finished = Some(result);
    }

    pub fn cancel(&self) {
        self.0.cancel.store(true, Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.0.cancel.load(Relaxed)
    }

    /// `Err` once cancelled: `?` it in a job's loops.
    pub fn check(&self) -> Result<(), String> {
        if self.cancelled() {
            Err("Stopped.".into())
        } else {
            Ok(())
        }
    }

    /// Runs `job` on a thread, finishing this progress with its result.
    pub fn spawn(
        &self,
        name: &str,
        job: impl FnOnce(&Progress) -> Result<String, String> + Send + 'static,
    ) {
        let progress = self.clone();
        let spawned = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&progress)))
                        .unwrap_or_else(|_| {
                            Err("Something went wrong (a bug): please report it.".into())
                        });
                progress.finish(result);
            });
        if let Err(e) = spawned {
            self.finish(Err(format!("Couldn't start: {e}")));
        }
    }
}

/// Bytes as people read them: "1.2 GB", "345 MB", "12 KB".
pub fn size_text(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{bytes} B")
    }
}

/// An HTTP agent for GitHub and the registry.
pub(crate) fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(60))
        .user_agent(concat!("horizon-nx-companion/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// GET `url` into memory, at most `limit` bytes.
pub(crate) fn fetch(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let response = agent().get(url).call().map_err(|e| match e {
        ureq::Error::Status(code, _) => format!("{url}: HTTP {code}"),
        ureq::Error::Transport(e) => format!("{url}: {e}"),
    })?;
    let mut body = Vec::new();
    response
        .into_reader()
        .take(limit + 1)
        .read_to_end(&mut body)
        .map_err(|e| format!("{url}: {e}"))?;
    if body.len() as u64 > limit {
        return Err(format!("{url}: larger than expected"));
    }
    Ok(body)
}

/// GET `url` into the file `to` (through `to.part`), reporting bytes to `progress`.
pub(crate) fn download(
    url: &str,
    to: &std::path::Path,
    progress: &Progress,
) -> Result<u64, String> {
    use std::io::{Read, Write};
    let response = agent().get(url).call().map_err(|e| match e {
        ureq::Error::Status(code, _) => format!("download: HTTP {code}"),
        ureq::Error::Transport(e) => format!("download: {e}"),
    })?;
    if let Some(len) = response
        .header("Content-Length")
        .and_then(|l| l.parse().ok())
    {
        progress.set_total(len);
    }
    let part = to.with_extension("part");
    let result = (|| {
        let mut input = response.into_reader();
        let mut out =
            std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
        let mut buf = vec![0u8; 1 << 20];
        let mut total = 0u64;
        loop {
            progress.check()?;
            let n = input.read(&mut buf).map_err(|e| format!("download: {e}"))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])
                .map_err(|e| format!("{}: {e}", part.display()))?;
            total += n as u64;
            progress.advance(n as u64);
        }
        out.sync_all().map_err(|e| e.to_string())?;
        Ok(total)
    })();
    match result {
        Ok(total) => {
            std::fs::rename(&part, to).map_err(|e| format!("{}: {e}", to.display()))?;
            Ok(total)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// SHA-256 of a file, as lowercase hex.
pub fn sha256_file(path: &std::path::Path) -> Result<String, String> {
    use sha2::Digest;
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
