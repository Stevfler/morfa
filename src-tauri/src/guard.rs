//! Runs blocking filesystem calls that may never return (a dead NFS/SMB/sshfs server leaves the
//! thread in uninterruptible sleep, which nothing can kill). Each call gets its own thread; when it
//! stops making progress the caller gets a "not responding" error and the thread is abandoned. How
//! many may be abandoned is bounded per mount and per subtree, so a dead server costs a few threads:
//! past the bound its calls fail at once without spawning, until a stuck call finally returns. A
//! subtree with nothing stuck keeps working whatever is stuck elsewhere, up to `hard_abandoned`.
use crate::{netfs, CmdResult};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError},
        Arc, LazyLock, Mutex,
    },
    time::{Duration, Instant},
};

/// Single metadata-style calls on a share.
pub const QUICK: Duration = Duration::from_secs(3);
/// No progress for this long while listing or reading: a slow share keeps going as long as it beats.
pub const STALL: Duration = Duration::from_secs(10);
/// Elsewhere the first call may have to wait for a sleeping disk to spin up.
const SPIN_UP: Duration = Duration::from_secs(15);
const SPIN_UP_STALL: Duration = Duration::from_secs(20);

pub fn quick_for(path: &str) -> Duration {
    if netfs::network_mount_of(Path::new(path)).is_some() { QUICK } else { SPIN_UP }
}
pub fn stall_for(path: &str) -> Duration {
    if netfs::network_mount_of(Path::new(path)).is_some() { STALL } else { SPIN_UP_STALL }
}
/// Prefix of the error the frontend turns into the "not responding" message.
pub const NOT_RESPONDING: &str = "not-responding";

const POLL: Duration = Duration::from_millis(25);

#[derive(Clone, Copy)]
pub struct Limits {
    /// Abandoned threads per key.
    pub abandoned: usize,
    /// Running threads per key, abandoned included: covers a burst before the first timeout fires.
    pub running: usize,
    /// Abandoned threads per group (see `key_of`).
    pub group: usize,
    /// Abandoned threads overall past which a group with stuck work gets no more threads.
    pub total: usize,
    /// The absolute ceilings, for groups with nothing stuck too: abandoned threads, and all threads.
    pub hard_abandoned: usize,
    pub hard_running: usize,
}
pub const LIMITS: Limits = Limits { abandoned: 4, running: 16, group: 8, total: 64, hard_abandoned: 128, hard_running: 256 };

/// Handed to the blocking closure; call `tick` whenever real progress happens.
#[derive(Clone)]
pub struct Beat {
    start: Instant,
    last: Arc<AtomicU64>,
}
impl Beat {
    pub fn tick(&self) {
        self.last.store(self.start.elapsed().as_millis() as u64, Ordering::Relaxed);
    }
    fn idle(&self) -> Duration {
        self.start.elapsed().saturating_sub(Duration::from_millis(self.last.load(Ordering::Relaxed)))
    }
}

#[derive(Default)]
struct Slot {
    running: usize,
    abandoned: usize,
}
#[derive(Default)]
struct Slots {
    keys: HashMap<String, Slot>,
    /// Abandoned threads per group.
    groups: HashMap<String, usize>,
    abandoned: usize,
    running: usize,
}

/// A key is `group\0rest`, or just the group.
fn group_of(key: &str) -> &str {
    key.split('\0').next().unwrap_or(key)
}

impl Slots {
    fn abandon(&mut self, key: &str) {
        self.abandoned += 1;
        *self.groups.entry(group_of(key).to_string()).or_default() += 1;
        if let Some(slot) = self.keys.get_mut(key) {
            slot.abandoned += 1;
        }
    }
}
#[derive(Default)]
pub struct Pool(Mutex<Slots>);

static POOL: LazyLock<Arc<Pool>> = LazyLock::new(Default::default);

impl Pool {
    fn lock(&self) -> std::sync::MutexGuard<'_, Slots> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Releases the worker's slot on return or panic. The flag is read under the pool lock, the same
/// lock the waiter sets it under, so a thread is never counted as abandoned after it has finished.
struct Release {
    pool: Arc<Pool>,
    key: String,
    abandoned: Arc<AtomicBool>,
}
impl Drop for Release {
    fn drop(&mut self) {
        let mut s = self.pool.lock();
        let was_abandoned = self.abandoned.load(Ordering::Relaxed);
        s.running -= 1;
        if was_abandoned {
            s.abandoned -= 1;
            let group = group_of(&self.key);
            if let Some(n) = s.groups.get_mut(group) {
                *n -= 1;
                if *n == 0 {
                    s.groups.remove(group);
                }
            }
        }
        if let Some(slot) = s.keys.get_mut(&self.key) {
            slot.running -= 1;
            slot.abandoned -= was_abandoned as usize;
            if slot.running == 0 {
                s.keys.remove(&self.key);
            }
        }
    }
}

/// Runs `f` on its own thread and waits, polling `cancel`. Blocks the calling thread for at most
/// `stall` past the last beat. `label` names the path in the error.
pub fn run_with<T, F>(pool: &Arc<Pool>, key: &str, label: &str, stall: Duration, limits: Limits, cancel: impl Fn() -> bool, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Beat) -> CmdResult<T> + Send + 'static,
{
    let not_responding = || format!("{NOT_RESPONDING}: {label}");
    {
        let mut s = pool.lock();
        let stuck_in_group = s.groups.get(group_of(key)).copied().unwrap_or(0);
        let refuse = s.abandoned >= limits.hard_abandoned
            || s.running >= limits.hard_running
            || (stuck_in_group > 0 && (stuck_in_group >= limits.group || s.abandoned >= limits.total))
            || s.keys.get(key).is_some_and(|k| k.abandoned >= limits.abandoned || k.running >= limits.running);
        if refuse {
            return Err(not_responding());
        }
        s.running += 1;
        s.keys.entry(key.to_string()).or_default().running += 1;
    }
    let beat = Beat { start: Instant::now(), last: Arc::new(AtomicU64::new(0)) };
    let abandoned = Arc::new(AtomicBool::new(false));
    let release = Release { pool: pool.clone(), key: key.to_string(), abandoned: abandoned.clone() };
    let (tx, rx) = mpsc::channel();
    let worker_beat = beat.clone();
    // On a spawn failure the closure, and with it `release`, is dropped: the slot is freed.
    std::thread::Builder::new()
        .name("fs-guard".into())
        .spawn(move || {
            let _release = release;
            let _ = tx.send(f(&worker_beat));
        })
        .map_err(|e| e.to_string())?;
    loop {
        match rx.recv_timeout(POLL) {
            Ok(r) => return r,
            Err(RecvTimeoutError::Disconnected) => return Err("filesystem worker failed".into()),
            Err(RecvTimeoutError::Timeout) => {}
        }
        if cancel() {
            // A walk that sees the flag exits by itself; it counts as stuck only if it doesn't.
            let (pool, key) = (pool.clone(), key.to_string());
            std::thread::spawn(move || {
                if matches!(rx.recv_timeout(stall), Err(RecvTimeoutError::Timeout)) {
                    let _ = give_up(&pool, &key, &rx, &abandoned);
                }
            });
            return Err("cancelled".into());
        }
        if beat.idle() >= stall {
            return give_up(pool, key, &rx, &abandoned).unwrap_or_else(|| Err(not_responding()));
        }
    }
}

/// Marks the worker abandoned, or returns its result if it finished just now.
fn give_up<T>(pool: &Pool, key: &str, rx: &mpsc::Receiver<CmdResult<T>>, abandoned: &AtomicBool) -> Option<CmdResult<T>> {
    let mut s = pool.lock();
    if let Ok(r) = rx.try_recv() {
        return Some(r);
    }
    // Sent nothing and gone (a panic) means the slot is already released.
    if s.keys.contains_key(key) && !matches!(rx.try_recv(), Err(mpsc::TryRecvError::Disconnected)) {
        s.abandon(key);
        abandoned.store(true, Ordering::Relaxed);
    }
    None
}

/// Where a path's stuck calls count: its mount on a network filesystem. Anything else counts per
/// path within a group, its first few components, because the mount it really reaches through a
/// symlink can't be known without touching it. So `~/dead-link/a` and `~/dead-link/b` run out
/// together, and `~/Documents` or any other sibling isn't affected.
fn key_of(path: &str) -> String {
    const GROUP_DEPTH: usize = 4;
    match netfs::network_mount_of(Path::new(path)) {
        Some(m) => format!("mount:{m}"),
        None => {
            let group: std::path::PathBuf = Path::new(path).components().take(GROUP_DEPTH).collect();
            format!("path:{}\0{path}", group.display())
        }
    }
}

/// Blocking form: for code already off the main thread (a `spawn_blocking` or a plain thread).
pub fn run_blocking<T, F>(path: &str, stall: Duration, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Beat) -> CmdResult<T> + Send + 'static,
{
    run_with(&POOL, &key_of(path), path, stall, LIMITS, || false, f)
}

/// Same, under an explicit key and bound; for calls that aren't about one path.
pub fn run_keyed<T, F>(key: &str, label: &str, stall: Duration, limits: Limits, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Beat) -> CmdResult<T> + Send + 'static,
{
    run_with(&POOL, key, label, stall, limits, || false, f)
}

/// `run_keyed`, also giving up as soon as `cancel` returns true.
pub fn run_keyed_cancel<T, F>(key: &str, label: &str, stall: Duration, limits: Limits, cancel: impl Fn() -> bool, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Beat) -> CmdResult<T> + Send + 'static,
{
    run_with(&POOL, key, label, stall, limits, cancel, f)
}

/// Waits on a pool thread: for at most `stall` past the last beat, so waiters can't pile up either.
pub async fn guarded<T, F>(path: &str, stall: Duration, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Beat) -> CmdResult<T> + Send + 'static,
{
    guarded_cancel(path, stall, || false, f).await
}

/// `guarded`, also giving up as soon as `cancel` returns true.
pub async fn guarded_cancel<T, F>(path: &str, stall: Duration, cancel: impl Fn() -> bool + Send + 'static, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Beat) -> CmdResult<T> + Send + 'static,
{
    let path = path.to_string();
    tauri::async_runtime::spawn_blocking(move || run_with(&POOL, &key_of(&path), &path, stall, LIMITS, cancel, f)).await.map_err(|e| e.to_string())?
}

/// Fails fast when `path`'s filesystem doesn't answer, before starting long work that has no
/// meaningful stall limit (copying, unpacking, walking). A missing file is the work's own error.
pub async fn probe(path: &str) -> CmdResult<()> {
    let p = path.to_string();
    guarded(path, quick_for(path), move |_| {
        let _ = std::fs::symlink_metadata(&p);
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    const L: Limits = Limits { abandoned: 2, running: 4, group: 3, total: 3, hard_abandoned: 6, hard_running: 12 };
    const SHORT: Duration = Duration::from_millis(40);

    /// A fake blocking fs call: returns only once released, like a server coming back.
    fn gate() -> (mpsc::Sender<()>, Arc<Mutex<mpsc::Receiver<()>>>) {
        let (tx, rx) = mpsc::channel();
        (tx, Arc::new(Mutex::new(rx)))
    }
    fn stuck(pool: &Arc<Pool>, key: &str, rx: &Arc<Mutex<mpsc::Receiver<()>>>) -> CmdResult<u32> {
        let rx = rx.clone();
        run_with(pool, key, key, SHORT, L, || false, move |_| {
            let _ = rx.lock().unwrap().recv();
            Ok(1)
        })
    }
    fn wait_until(what: &str, cond: impl Fn() -> bool) {
        let t = Instant::now();
        while !cond() {
            assert!(t.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn passes_through_results() {
        let p = Arc::new(Pool::default());
        assert_eq!(run_with(&p, "k", "k", SHORT, L, || false, |_| Ok(7)), Ok(7));
        assert_eq!(run_with::<u8, _>(&p, "k", "k", SHORT, L, || false, |_| Err("nope".into())), Err("nope".to_string()));
        assert!(p.lock().keys.is_empty(), "slots freed");
    }

    #[test]
    fn hang_times_out_promptly_with_a_recognisable_error() {
        let p = Arc::new(Pool::default());
        let (release, rx) = gate();
        let t = Instant::now();
        let e = stuck(&p, "k", &rx).unwrap_err();
        assert!(t.elapsed() < Duration::from_secs(2));
        assert!(e.starts_with(NOT_RESPONDING), "{e}");
        assert_eq!(p.lock().abandoned, 1);
        drop(release);
        wait_until("worker exit", || p.lock().abandoned == 0 && p.lock().keys.is_empty());
    }

    #[test]
    fn abandoned_threads_are_bounded_per_key_and_recover() {
        let p = Arc::new(Pool::default());
        let (release, rx) = gate();
        let spawned = Arc::new(AtomicUsize::new(0));
        let attempt = |key: &str| {
            let (s, rx) = (spawned.clone(), rx.clone());
            run_with(&p, key, key, SHORT, L, || false, move |_| {
                s.fetch_add(1, Ordering::SeqCst);
                let _ = rx.lock().unwrap().recv();
                Ok(())
            })
        };
        for _ in 0..2 {
            attempt("dead").unwrap_err();
        }
        // Past the bound: an immediate refusal, with no new thread.
        let t = Instant::now();
        assert!(attempt("dead").unwrap_err().starts_with(NOT_RESPONDING));
        assert!(t.elapsed() < SHORT, "refusal must not wait");
        wait_until("workers to start", || spawned.load(Ordering::SeqCst) >= 1);
        assert_eq!(p.lock().keys["dead"].running, 2);
        // Another mount isn't affected.
        assert_eq!(run_with(&p, "alive", "alive", SHORT, L, || false, |_| Ok(1)), Ok(1));
        // The server comes back: threads return, and the key works again.
        drop(release);
        wait_until("recovery", || p.lock().abandoned == 0);
        assert_eq!(run_with(&p, "dead", "dead", SHORT, L, || false, |_| Ok(2)), Ok(2));
    }

    #[test]
    fn group_bound_covers_many_paths_under_one_dead_link() {
        let p = Arc::new(Pool::default());
        let (release, rx) = gate();
        for k in ["link\0a", "link\0b", "link\0c"] {
            stuck(&p, k, &rx).unwrap_err();
        }
        let t = Instant::now();
        assert!(run_with(&p, "link\0d", "d", SHORT, L, || false, |_| Ok(())).unwrap_err().starts_with(NOT_RESPONDING));
        assert!(t.elapsed() < SHORT);
        assert_eq!(run_with(&p, "other\0x", "x", SHORT, L, || false, |_| Ok(1)), Ok(1), "a sibling is unaffected");
        drop(release);
        wait_until("recovery", || p.lock().abandoned == 0);
    }

    /// Stuck work everywhere must not starve a group with nothing stuck, and threads stay capped.
    #[test]
    fn saturated_groups_dont_starve_healthy_ones_and_threads_are_capped() {
        let p = Arc::new(Pool::default());
        let (release, rx) = gate();
        for i in 0..40 {
            assert!(stuck(&p, &format!("g{i}"), &rx).unwrap_err().starts_with(NOT_RESPONDING));
        }
        let (abandoned, running) = {
            let s = p.lock();
            (s.abandoned, s.running)
        };
        assert_eq!((abandoned, running), (L.hard_abandoned, L.hard_abandoned), "fresh groups admitted up to the ceiling, no further");
        drop(release);
        wait_until("recovery", || p.lock().running == 0);
    }

    /// The same on the shared pool: a real listing still works past the soft bound.
    #[test]
    fn local_listing_survives_many_abandoned_calls() {
        let (release, rx) = gate();
        let soft = LIMITS.total + 8;
        let attempts: Vec<_> = (0..soft)
            .map(|i| {
                let rx = rx.clone();
                std::thread::spawn(move || {
                    run_with(&POOL, &format!("saturate-{i}\0x"), "x", SHORT, LIMITS, || false, move |_| {
                        let _ = rx.lock().unwrap().recv();
                        Ok(())
                    })
                })
            })
            .collect();
        for a in attempts {
            a.join().unwrap().unwrap_err();
        }
        assert!(POOL.lock().abandoned >= LIMITS.total);
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        assert!(tauri::async_runtime::block_on(crate::fsinfo::list_dir(dir)).is_ok());
        drop(release);
        wait_until("recovery", || POOL.lock().groups.keys().all(|g| !g.starts_with("saturate-")));
    }

    #[test]
    fn running_bound_refuses_a_burst_before_any_timeout() {
        let p = Arc::new(Pool::default());
        let (release, rx) = gate();
        let long = Duration::from_secs(30);
        let mut waiters = vec![];
        for _ in 0..L.running {
            let (p, rx) = (p.clone(), rx.clone());
            waiters.push(std::thread::spawn(move || {
                run_with(&p, "k", "k", long, L, || false, move |_| {
                    let _ = rx.lock().unwrap().recv();
                    Ok(())
                })
            }));
        }
        wait_until("burst", || p.lock().keys.get("k").is_some_and(|s| s.running == L.running));
        assert!(run_with(&p, "k", "k", long, L, || false, |_| Ok(())).unwrap_err().starts_with(NOT_RESPONDING));
        drop(release);
        for w in waiters {
            w.join().unwrap().unwrap();
        }
    }

    #[test]
    fn beats_keep_slow_progress_alive() {
        let p = Arc::new(Pool::default());
        let r = run_with(&p, "k", "k", Duration::from_millis(150), L, || false, |b| {
            for _ in 0..10 {
                std::thread::sleep(Duration::from_millis(50)); // 500 ms in all, over 3x the limit
                b.tick();
            }
            Ok("done")
        });
        assert_eq!(r, Ok("done"));
    }

    #[test]
    fn cancel_abandons_without_waiting_for_the_stall() {
        let p = Arc::new(Pool::default());
        let (release, rx) = gate();
        let t = Instant::now();
        let e = run_with(&p, "k", "k", Duration::from_secs(30), L, || true, move |_| {
            let _ = rx.lock().unwrap().recv();
            Ok(())
        });
        assert_eq!(e, Err("cancelled".to_string()));
        assert!(t.elapsed() < Duration::from_secs(2));
        assert_eq!(p.lock().abandoned, 0, "not stuck until it fails to stop");
        drop(release);
        wait_until("worker exit", || p.lock().running == 0);
    }

    #[test]
    fn cancelled_call_that_never_stops_counts_as_abandoned_after_the_stall() {
        let p = Arc::new(Pool::default());
        let (release, rx) = gate();
        let e = run_with(&p, "k", "k", SHORT, L, || true, move |_| {
            let _ = rx.lock().unwrap().recv();
            Ok(())
        });
        assert_eq!(e, Err("cancelled".to_string()));
        wait_until("late abandon", || p.lock().abandoned == 1);
        drop(release);
        wait_until("worker exit", || p.lock().abandoned == 0 && p.lock().running == 0);
    }

    #[test]
    fn panic_frees_the_slot() {
        let p = Arc::new(Pool::default());
        let r: CmdResult<()> = run_with(&p, "k", "k", SHORT, L, || false, |_| panic!("expected in this test"));
        assert!(r.is_err());
        wait_until("slot release", || p.lock().keys.is_empty());
    }

    /// The real thing: opening a FIFO with no writer blocks in the kernel, like a dead mount.
    #[cfg(unix)]
    #[test]
    fn real_blocking_open_on_a_fifo() {
        use std::os::unix::ffi::OsStrExt;
        let f = std::env::temp_dir().join(format!("morfa-guard-fifo-{}", std::process::id()));
        let _ = std::fs::remove_file(&f);
        let c = std::ffi::CString::new(f.as_os_str().as_bytes()).unwrap();
        // SAFETY: a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let p = Arc::new(Pool::default());
        let f2 = f.clone();
        let t = Instant::now();
        let e = run_with(&p, "fifo", "fifo", Duration::from_millis(200), L, || false, move |_| std::fs::read(&f2).map_err(|e| e.to_string())).unwrap_err();
        assert!(e.starts_with(NOT_RESPONDING), "{e}");
        assert!(t.elapsed() < Duration::from_secs(2));
        // A writer unblocks the abandoned read, which then gives its slot back.
        let w = std::fs::OpenOptions::new().write(true).open(&f).unwrap();
        drop(w);
        wait_until("abandoned read to return", || p.lock().abandoned == 0);
        std::fs::remove_file(&f).unwrap();
    }
}
