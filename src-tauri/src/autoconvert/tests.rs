use super::core::*;
use super::filter::{self, Spec};
use super::ledger::{self, Ledger};
use super::rules::{self, Inspect, PathEnv, Recipe};
use crate::convert::{routes::Via, ConvertOpts};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::{AtomicU64, AtomicUsize, Ordering}, Arc, Mutex},
};

static N: AtomicU64 = AtomicU64::new(0);

struct Tmp(PathBuf);
impl Tmp {
    fn new() -> Self {
        let d = std::env::temp_dir().join(format!("morfa-auto-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&d).unwrap();
        Tmp(d)
    }
    fn dir(&self, name: &str) -> PathBuf {
        let d = self.0.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::canonicalize(d).unwrap()
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Every local path is fine, unless a test says otherwise.
#[derive(Default)]
struct Env {
    network: Vec<PathBuf>,
    protected: Vec<PathBuf>,
}
impl PathEnv for Env {
    fn inspect(&self, p: &Path) -> Inspect {
        let real = std::fs::canonicalize(p).ok();
        Inspect { is_dir: p.is_dir(), real, network: self.network.iter().any(|n| p.starts_with(n)), dead: false }
    }
    fn protected(&self, real: &Path) -> bool {
        self.protected.iter().any(|n| real.starts_with(n))
    }
}

fn opts(format: &str, out: Option<&Path>) -> ConvertOpts {
    ConvertOpts { format: format.into(), quality: 80, resize: None, strip_metadata: false, out_dir: out.map(|p| p.to_string_lossy().into_owned()), name_template: "{name}".into(), max_bytes: None }
}
fn recipe(id: &str, format: &str, out: Option<&Path>) -> Recipe {
    Recipe { id: id.into(), name: id.into(), opts: opts(format, out) }
}

/// Every image route exists, without any engine installed.
fn via(e: &str, f: &str) -> Option<Via> {
    const IMG: [&str; 4] = ["png", "jpg", "webp", "gif"];
    (IMG.contains(&e) && IMG.contains(&f)).then_some(Via::Image)
}

/// Stands in for the Trash in tests: the real one is the user's.
fn to_scratch(p: &Path) -> bool {
    std::fs::remove_file(p).is_ok()
}

/// The fake encoder: writes one never-overwriting output, like the real publish step, and counts.
struct Fake {
    calls: Arc<AtomicUsize>,
    fail: Arc<Mutex<Option<String>>>,
}
impl Fake {
    fn new() -> Self {
        Fake { calls: Default::default(), fail: Default::default() }
    }
    fn encoder(&self) -> Box<Encoder> {
        let (calls, fail) = (self.calls.clone(), self.fail.clone());
        Box::new(move |src: &Path, o: &ConvertOpts, _: &dyn Fn() -> bool| {
            calls.fetch_add(1, Ordering::SeqCst);
            if let Some(e) = fail.lock().unwrap().clone() {
                return Err(e);
            }
            let dest = crate::convert::naming::dest_for(src, o.out_dir.as_deref(), &o.name_template, &o.format);
            std::fs::write(&dest, std::fs::read(src).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            Ok(vec![dest])
        })
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

/// A rule engine on a fake clock, driven the way the real driver does it.
struct Rig {
    core: Mutex<Core>,
    now: Arc<AtomicU64>,
    enc: Box<Encoder>,
}
impl Rig {
    fn new(dir: Option<PathBuf>, recipes: Vec<Recipe>, fake: &Fake) -> Self {
        let mut c = Core::load(dir);
        c.sync_recipes(recipes, &Env::default());
        Rig { core: Mutex::new(c), now: Arc::new(AtomicU64::new(1_000_000)), enc: fake.encoder() }
    }
    fn c(&self) -> std::sync::MutexGuard<'_, Core> {
        self.core.lock().unwrap()
    }
    fn add_on(&self, folder: &Path, recipe: &str, sub: bool) -> String {
        let id = format!("r{}", N.fetch_add(1, Ordering::SeqCst));
        self.c().add(folder.to_string_lossy().into_owned(), recipe.into(), sub, id.clone(), &Env::default()).unwrap();
        self.c().set_enabled(&id, true, &Env::default()).unwrap();
        id
    }
    fn at(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
    /// One driver tick after `ms` passes: scan what's due, run at most one job.
    fn tick(&self, ms: u64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
        let now = self.at();
        let scans = self.c().due_scans(now);
        for s in scans {
            let res = scan(Path::new(&s.folder), s.subfolders, &s.spec, &s.own, &via, &|| {});
            self.c().apply_scan(&s.rule_id, res, now);
        }
        let job = self.c().next_job(now);
        if let Some(job) = job {
            let clock = self.now.clone();
            run_job(&self.core, job, &*self.enc, &via, &to_scratch, &move || clock.load(Ordering::SeqCst));
        }
    }
    /// Long enough for anything stable and any scan interval, several times over.
    fn settle(&self) {
        let ids: Vec<String> = self.c().rules.iter().map(|r| r.id.clone()).collect();
        for id in ids {
            self.c().mark_dirty(&id);
        }
        for _ in 0..12 {
            self.tick(31_000);
        }
    }
    fn view(&self, id: &str) -> RuleView {
        self.c().overview().rules.into_iter().find(|r| r.id == id).unwrap()
    }
}

fn names(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(d).unwrap().map(|x| x.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

// --- ignore rules and formats

#[test]
fn ignores_partial_hidden_and_our_own() {
    for n in ["a.crdownload", "a.jpg.part", "a.DOWNLOAD", "x.tmp", ".morfa-convert-1-2.png", ".morfa-x", ".hidden.png", "~$doc.docx"] {
        assert!(filter::ignored_name(n), "{n}");
    }
    for n in ["a.png", "part.png", "tmp", "photo.tmp.png"] {
        assert!(!filter::ignored_name(n), "{n}");
    }
}

#[test]
fn only_files_the_recipe_can_take() {
    let spec = |f: &str, next, limited| Spec { format: f.into(), next_to_source: next, limited };
    assert!(filter::eligible(Path::new("/x/a.png"), &spec("webp", true, false), &via));
    assert!(!filter::eligible(Path::new("/x/a.txt"), &spec("webp", true, false), &via), "no route");
    assert!(!filter::eligible(Path::new("/x/noext"), &spec("webp", true, false), &via));
    assert!(!filter::eligible(Path::new("/x/a.webp"), &spec("webp", true, false), &via), "target format next to source");
    assert!(!filter::eligible(Path::new("/x/a.JPEG"), &spec("jpg", true, false), &via), "spellings of the target too");
    assert!(filter::eligible(Path::new("/x/a.webp"), &spec("webp", false, false), &via), "fine when output goes elsewhere");
    let any = |_: &str, _: &str| Some(Via::Video);
    assert!(!filter::eligible(Path::new("/x/a.mp4"), &spec("gif", false, true), &any), "a size limit is for images only");
}

// --- stability

#[test]
fn stability_restarts_on_change_and_needs_content() {
    let t0 = 100;
    let since = filter::observed_since(None, 10, 5, t0);
    assert!(!filter::is_stable(since, filter::STABLE_MS, t0 + filter::STABLE_MS - 1));
    assert!(filter::is_stable(since, filter::STABLE_MS, t0 + filter::STABLE_MS));
    assert_eq!(filter::quiet_for("/x/a.png"), filter::STABLE_MS);
    assert_eq!(filter::quiet_for("/x/a.mp3"), filter::UNVERIFIED_QUIET_MS, "nothing but time to go on");
    assert_eq!(filter::observed_since(Some((10, 5, since)), 10, 5, t0 + 900), since);
    assert_eq!(filter::observed_since(Some((10, 5, since)), 11, 5, t0 + 900), t0 + 900, "size grew");
    assert_eq!(filter::observed_since(Some((10, 5, since)), 10, 6, t0 + 900), t0 + 900, "mtime moved");
}

#[test]
fn a_growing_file_waits_until_it_stops() {
    let t = Tmp::new();
    let d = t.dir("in");
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&t.dir("out")))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    let f = d.join("a.png");
    let mut bytes = vec![1u8; 10];
    for _ in 0..8 {
        bytes.push(1);
        std::fs::write(&f, &bytes).unwrap();
        rig.c().mark_dirty(&id);
        rig.tick(1000);
        assert_eq!(fake.calls(), 0, "still being written");
    }
    rig.settle();
    assert_eq!(fake.calls(), 1);
}

// --- baseline

#[test]
fn existing_files_are_left_alone_until_opted_in() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    std::fs::write(d.join("old.png"), b"x").unwrap();
    std::fs::write(d.join("note.txt"), b"x").unwrap();
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    assert_eq!(fake.calls(), 0);
    assert_eq!(rig.view(&id).backlog, 1, "the preview count: only what the recipe can take");
    std::fs::write(d.join("new.png"), b"y").unwrap();
    rig.settle();
    assert_eq!(fake.calls(), 1);
    assert_eq!(names(&out), ["new.webp"]);
    rig.c().process_existing(&id, 1);
    rig.settle();
    assert_eq!(names(&out), ["new.webp", "old.webp"]);
    assert_eq!(rig.view(&id).backlog, 0);
}

// --- loop prevention

/// Whatever the layout, the number of conversions stops growing, across restarts too.
fn assert_no_loop(dir: &Path, recipes: Vec<Recipe>, rules_of: &[(&Path, &str, bool)], drop_in: &Path, want: usize) {
    let fake = Fake::new();
    let mut ids = vec![];
    {
        let rig = Rig::new(Some(dir.to_path_buf()), recipes.clone(), &fake);
        ids.extend(rules_of.iter().map(|(f, r, s)| rig.add_on(f, r, *s)));
        rig.settle();
        std::fs::write(drop_in.join("a.png"), b"one").unwrap();
        for _ in 0..4 {
            rig.settle();
        }
        assert_eq!(fake.calls(), want, "first run");
    }
    for _ in 0..3 {
        // A restart, then everything opted in: outputs and handled files are still not touched.
        let rig = Rig::new(Some(dir.to_path_buf()), recipes.clone(), &fake);
        rig.settle();
        for id in &ids {
            rig.c().process_existing(id, 1);
        }
        rig.settle();
        assert_eq!(fake.calls(), want, "after a restart");
    }
}

#[test]
fn next_to_source_never_retriggers() {
    let t = Tmp::new();
    let d = t.dir("in");
    assert_no_loop(&t.dir("cfg"), vec![recipe("r", "webp", None)], &[(&d, "r", false)], &d, 1);
    assert_eq!(names(&d), ["a.png", "a.webp"]);
}

#[test]
fn target_equal_to_source_next_to_source_does_nothing_at_all() {
    let t = Tmp::new();
    let d = t.dir("in");
    assert_no_loop(&t.dir("cfg"), vec![recipe("r", "png", None)], &[(&d, "r", false)], &d, 0);
    assert_eq!(names(&d), ["a.png"]);
}

#[test]
fn target_equal_to_source_with_an_output_folder_runs_once() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    assert_no_loop(&t.dir("cfg"), vec![recipe("r", "png", Some(&out))], &[(&d, "r", false)], &d, 1);
    assert_eq!(names(&out), ["a.png"]);
}

#[test]
fn next_to_source_in_subfolders_never_retriggers() {
    let t = Tmp::new();
    let d = t.dir("in");
    let sub = t.dir("in/deep");
    assert_no_loop(&t.dir("cfg"), vec![recipe("r", "webp", None)], &[(&d, "r", true)], &sub, 1);
    assert_eq!(names(&sub), ["a.png", "a.webp"]);
}

#[test]
fn two_rules_feeding_each_other_stop() {
    let t = Tmp::new();
    let (a, b) = (t.dir("a"), t.dir("b"));
    let recipes = vec![recipe("to-b", "webp", Some(&b)), recipe("to-a", "png", Some(&a))];
    assert_no_loop(&t.dir("cfg"), recipes, &[(&a, "to-b", false), (&b, "to-a", false)], &a, 1);
    assert_eq!(names(&b), ["a.webp"]);
}

#[test]
fn a_new_version_of_a_source_counts_but_its_outputs_do_not() {
    let t = Tmp::new();
    let d = t.dir("in");
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", None)], &fake);
    rig.add_on(&d, "r", false);
    rig.settle();
    std::fs::write(d.join("a.png"), b"1").unwrap();
    rig.settle();
    std::fs::write(d.join("a.png"), b"22").unwrap();
    rig.settle();
    assert_eq!(fake.calls(), 2);
    assert_eq!(names(&d), ["a (converted).webp", "a.png", "a.webp"], "and nothing overwritten");
    assert_eq!(std::fs::read(d.join("a.png")).unwrap(), b"22", "source as it was");
}

// --- dedupe

#[test]
fn dedupe_survives_a_restart_and_is_bounded() {
    let t = Tmp::new();
    let f = t.0.join("ledger.jsonl");
    let k = ledger::key("r", "/x/a.png", 1, 2);
    {
        let mut l = Ledger::open(Some(f.clone()), true);
        l.mark_handled(&k, "");
        l.mark_produced("/x/a.webp", "");
    }
    std::fs::OpenOptions::new().append(true).open(&f).and_then(|mut h| std::io::Write::write_all(&mut h, b"[\"k\",\"torn")).unwrap();
    let mut l = Ledger::open(Some(f.clone()), true);
    assert!(l.handled(&k) && l.produced("/x/a.webp"), "a torn line costs nothing");
    assert!(!l.handled(&ledger::key("r", "/x/a.png", 2, 2)), "another size is another version");
    l.forget(&k);
    assert!(!Ledger::open(Some(f.clone()), true).handled(&k));

    let mut b = Ledger::with_cap(Some(t.0.join("b.jsonl")), 5);
    for i in 0..5000 {
        b.mark_handled(&format!("k{i}"), "");
        b.mark_produced(&format!("p{i}"), "");
    }
    assert!(b.handled("k4999") && !b.handled("k0") && b.produced("p4999") && !b.produced("p0"));
    let lines = std::fs::read_to_string(t.0.join("b.jsonl")).unwrap().lines().count();
    assert!(lines <= 2 * 5 + 1002, "compacted: {lines}");
    let again = Ledger::with_cap(Some(t.0.join("b.jsonl")), 5);
    assert!(again.handled("k4999") && !again.handled("k0"));
    assert!(again.lossy(), "and it remembers having forgotten");
}

#[test]
fn a_crash_mid_file_does_not_repeat_it() {
    let t = Tmp::new();
    let (cfg, d, out) = (t.dir("cfg"), t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(Some(cfg.clone()), vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    std::fs::write(d.join("a.png"), b"1").unwrap();
    rig.c().mark_dirty(&id);
    rig.tick(1000);
    rig.c().mark_dirty(&id);
    rig.tick(4000);
    let job = rig.c().next_job(rig.at());
    let job = match job {
        Some(j) => j,
        None => {
            rig.c().mark_dirty(&id);
            rig.tick(4000);
            assert_eq!(fake.calls(), 1, "already ran");
            return;
        }
    };
    rig.c().began(&job, rig.at());
    drop(rig); // the process dies here
    let fake2 = Fake::new();
    let rig = Rig::new(Some(cfg), vec![recipe("r", "webp", Some(&out))], &fake2);
    rig.settle();
    rig.c().process_existing(&id, 1);
    rig.settle();
    assert_eq!(fake2.calls(), 0);
}

// --- rate cap, mass drop

#[test]
fn a_slow_flood_hits_the_rate_cap_and_pauses_until_answered() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    let total = RATE_PER_MIN + 10;
    for i in 0..total {
        std::fs::write(d.join(format!("f{i}.png")), b"x").unwrap();
    }
    rig.c().mark_dirty(&id);
    rig.tick(0);
    for _ in 0..(total * 2) {
        rig.c().mark_dirty(&id);
        rig.tick(1000);
        if rig.view(&id).hold.is_some() {
            break;
        }
    }
    assert_eq!(rig.view(&id).hold, Some(Hold::Rate));
    assert_eq!(fake.calls(), RATE_PER_MIN, "no more than the cap in the window");
    rig.c().resolve(&id, true);
    for _ in 0..3 {
        rig.c().mark_dirty(&id);
        rig.tick(100);
    }
    assert!(rig.view(&id).hold.is_none());
    assert_eq!(fake.calls(), RATE_PER_MIN, "the window still applies after continuing");
    rig.settle();
    for _ in 0..10 {
        rig.settle();
    }
    assert_eq!(fake.calls(), total);
    assert_eq!(rig.view(&id).hold, None);
}

#[test]
fn a_rate_pause_does_not_lift_by_itself() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    for i in 0..(RATE_PER_MIN + 5) {
        std::fs::write(d.join(format!("f{i}.png")), b"x").unwrap();
    }
    for _ in 0..(RATE_PER_MIN + 10) {
        rig.c().mark_dirty(&id);
        rig.tick(1000);
    }
    assert_eq!(rig.view(&id).hold, Some(Hold::Rate));
    let before = fake.calls();
    rig.settle();
    assert_eq!(fake.calls(), before, "the window has long passed, and it still waits for an answer");
}

#[test]
fn a_mass_drop_asks_first_and_can_be_declined() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    for i in 0..10_000 {
        std::fs::write(d.join(format!("f{i}.png")), b"x").unwrap();
    }
    rig.c().mark_dirty(&id);
    rig.tick(0);
    let v = rig.view(&id);
    assert_eq!((v.hold, v.pending), (Some(Hold::Mass), 10_000));
    for _ in 0..3 {
        rig.tick(31_000);
    }
    assert_eq!(fake.calls(), 0, "nothing before the answer");
    rig.c().resolve(&id, false);
    rig.settle();
    let v = rig.view(&id);
    assert_eq!((fake.calls(), v.hold, v.pending, v.backlog), (0, None, 0, 10_000), "declined: treated as already there");
}

#[test]
fn a_mass_drop_continues_at_the_capped_rate_when_allowed() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    for i in 0..MASS_ASK {
        std::fs::write(d.join(format!("f{i}.png")), b"x").unwrap();
    }
    rig.c().mark_dirty(&id);
    rig.tick(0);
    assert_eq!(rig.view(&id).hold, Some(Hold::Mass));
    rig.c().resolve(&id, true);
    for _ in 0..200 {
        rig.c().mark_dirty(&id);
        rig.tick(1000);
    }
    assert_eq!(rig.view(&id).hold, None, "no second question for the same files");
    assert!(fake.calls() <= RATE_PER_MIN * 4 + 1 && fake.calls() >= RATE_PER_MIN, "{}", fake.calls());
}

// --- retries

#[test]
fn failures_retry_a_few_times_then_are_listed_and_left() {
    let t = Tmp::new();
    let (cfg, d, out) = (t.dir("cfg"), t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    *fake.fail.lock().unwrap() = Some("engine exploded".into());
    let rig = Rig::new(Some(cfg.clone()), vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    std::fs::write(d.join("bad.png"), b"x").unwrap();
    for _ in 0..4 {
        rig.settle();
    }
    assert_eq!(fake.calls(), MAX_ATTEMPTS as usize);
    let v = rig.view(&id);
    assert_eq!((v.failed, v.failures.len(), v.pending), (1, 1, 0));
    assert_eq!(v.failures[0].error, "engine exploded");
    assert!(v.failures[0].path.ends_with("bad.png"));
    let events = rig.c().take_events();
    assert_eq!(events.iter().map(|e| e.failed.len()).sum::<usize>(), 1, "one summary, not three");
    drop(rig);
    let rig = Rig::new(Some(cfg), vec![recipe("r", "webp", Some(&out))], &fake);
    rig.settle();
    rig.c().process_existing(&id, 1);
    rig.settle();
    assert_eq!(fake.calls(), MAX_ATTEMPTS as usize, "nor after a restart");
    assert_eq!(rig.view(&id).failures.len(), 1, "still listed");
}

#[test]
fn a_hopeless_failure_is_not_retried() {
    let t = Tmp::new();
    let d = t.dir("in");
    let fake = Fake::new();
    *fake.fail.lock().unwrap() = Some("NO_ENGINE:ffmpeg".into());
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&t.dir("out")))], &fake);
    rig.add_on(&d, "r", false);
    rig.settle();
    std::fs::write(d.join("a.png"), b"x").unwrap();
    rig.settle();
    assert_eq!(fake.calls(), 1);
}

// --- rule validation

#[test]
fn refuses_what_must_not_be_watched() {
    let t = Tmp::new();
    let (d, inner, other) = (t.dir("in"), t.dir("in/sub"), t.dir("other"));
    let ok = Env::default();
    let r = |o: Option<&Path>| recipe("r", "webp", o);
    assert_eq!(rules::check_rule("morfa://cloud/x/y", false, Some(&r(None)), &ok), Err("pseudo"));
    assert_eq!(rules::check_rule("morfa://drives", false, Some(&r(None)), &ok), Err("pseudo"));
    assert_eq!(rules::check_rule("relative/path", false, Some(&r(None)), &ok), Err("pseudo"));
    assert_eq!(rules::check_rule(&format!("{}/../in", d.display()), false, Some(&r(None)), &ok), Err("pseudo"));
    let f = d.to_string_lossy().into_owned();
    assert_eq!(rules::check_rule(&f, false, None, &ok), Err("recipeMissing"));
    assert_eq!(rules::check_rule(&f, false, Some(&r(None)), &Env { network: vec![d.clone()], ..Default::default() }), Err("network"));
    assert_eq!(rules::check_rule(&f, false, Some(&r(None)), &Env { protected: vec![d.clone()], ..Default::default() }), Err("protected"));
    assert_eq!(rules::check_rule(&format!("{f}/nope"), false, Some(&r(None)), &ok), Err("missing"));
    // Output
    assert_eq!(rules::check_rule(&f, false, Some(&r(None)), &ok), Ok(()), "next to the source");
    assert_eq!(rules::check_rule(&f, false, Some(&r(Some(&other))), &ok), Ok(()));
    assert_eq!(rules::check_rule(&f, false, Some(&r(Some(&d))), &ok), Err("outputSame"));
    assert_eq!(rules::check_rule(&f, false, Some(&r(Some(&inner))), &ok), Ok(()), "a subfolder that isn't watched");
    assert_eq!(rules::check_rule(&f, true, Some(&r(Some(&inner))), &ok), Err("outputInside"));
    assert_eq!(rules::check_rule(&f, false, Some(&r(Some(&other))), &Env { network: vec![other.clone()], ..Default::default() }), Err("outputNetwork"));
    assert_eq!(rules::check_rule(&f, false, Some(&r(Some(&other))), &Env { protected: vec![other.clone()], ..Default::default() }), Err("outputProtected"));
    let mut bad = r(None);
    bad.opts.name_template = "D:evil".into();
    assert_eq!(rules::check_rule(&f, false, Some(&bad), &ok), Err("recipeBad"));
    #[cfg(unix)]
    {
        let link = t.0.join("link");
        std::os::unix::fs::symlink(&d, &link).unwrap();
        assert_eq!(rules::check_rule(&f, false, Some(&r(Some(&link))), &ok), Err("outputSame"), "through a symlink");
    }
}

#[test]
fn real_rules_protect_system_folders() {
    let env = rules::RealEnv;
    let r = recipe("r", "webp", None);
    for p in ["/", "/usr", "/etc", "/usr/share"] {
        assert_eq!(rules::check_rule(p, false, Some(&r), &env), Err("protected"), "{p}");
    }
    let home = dirs::home_dir().unwrap();
    assert_eq!(rules::check_rule(&home.to_string_lossy(), false, Some(&r), &env), Err("protected"));
    assert_eq!(rules::check_rule(&home.join(".ssh").to_string_lossy(), false, Some(&r), &env), Err("protected"));
}

#[test]
fn rules_are_off_until_enabled_and_a_deleted_recipe_flags_them() {
    let t = Tmp::new();
    let (cfg, d) = (t.dir("cfg"), t.dir("in"));
    let fake = Fake::new();
    let rig = Rig::new(Some(cfg.clone()), vec![recipe("r", "webp", None)], &fake);
    assert!(rig.c().rules.is_empty(), "nothing exists by itself");
    let f = d.to_string_lossy().into_owned();
    rig.c().add(f.clone(), "r".into(), false, "a".into(), &Env::default()).unwrap();
    assert!(!rig.view("a").enabled && !rig.view("a").subfolders);
    assert_eq!(rig.c().add(f, "r".into(), true, "b".into(), &Env::default()), Err("duplicate".to_string()));
    std::fs::write(d.join("x.png"), b"1").unwrap();
    rig.settle();
    assert_eq!(fake.calls(), 0);
    rig.c().set_enabled("a", true, &Env::default()).unwrap();
    rig.settle();
    rig.c().sync_recipes(vec![], &Env::default());
    let v = rig.view("a");
    assert_eq!((v.enabled, v.flag.as_deref()), (false, Some("recipeMissing")));
    assert_eq!(rig.c().set_enabled("a", true, &Env::default()), Err("recipeMissing".to_string()));
    drop(rig);
    let rig = Rig::new(Some(cfg), vec![], &fake);
    assert_eq!(rig.view("a").flag.as_deref(), Some("recipeMissing"), "stored");
}

#[test]
fn unreadable_rules_are_set_aside_and_nothing_runs() {
    let t = Tmp::new();
    let cfg = t.dir("cfg");
    std::fs::write(cfg.join("autoconvert.json"), "{ not json").unwrap();
    let c = Core::load(Some(cfg.clone()));
    assert!(c.rules.is_empty());
    assert!(cfg.join("autoconvert.json.bad").exists());
}

#[test]
fn recipes_parse_as_the_ui_sends_them() {
    let r: Recipe = serde_json::from_str(r#"{"id":"a","name":"N","format":"webp","quality":80,"resize":{"mode":"max","value":100},"stripMetadata":true,"maxBytes":2048,"outDir":null,"nameTemplate":"{name}"}"#).unwrap();
    assert_eq!((r.id.as_str(), r.opts.quality, r.opts.max_bytes, r.opts.out_dir), ("a", 80, Some(2048), None));
}

#[cfg(unix)]
#[test]
fn a_file_that_will_not_open_is_listed_not_tried_forever() {
    use std::os::unix::fs::PermissionsExt;
    let t = Tmp::new();
    let d = t.dir("in");
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&t.dir("out")))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    let f = d.join("locked.png");
    std::fs::write(&f, b"x").unwrap();
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&f).is_ok() {
        return; // running as root
    }
    for _ in 0..(MAX_OPEN_TRIES * 2) {
        rig.c().mark_dirty(&id);
        rig.tick(filter::STABLE_MS);
    }
    let v = rig.view(&id);
    assert_eq!((fake.calls(), v.failed, v.pending), (0, 1, 0));
    assert_eq!(v.failures[0].error, "CANT_OPEN");
}

// --- stopping, disappearing

#[test]
fn disabling_drops_the_queue_and_cancels_what_runs() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    std::fs::write(d.join("a.png"), b"1").unwrap();
    std::fs::write(d.join("b.png"), b"1").unwrap();
    rig.c().mark_dirty(&id);
    rig.tick(0);
    rig.c().mark_dirty(&id);
    rig.now.fetch_add(filter::STABLE_MS, Ordering::SeqCst);
    let now = rig.at();
    let scans = rig.c().due_scans(now);
    for s in scans {
        let res = scan(Path::new(&s.folder), s.subfolders, &s.spec, &s.own, &via, &|| {});
        rig.c().apply_scan(&s.rule_id, res, now);
    }
    let job = rig.c().next_job(now).expect("a job");
    assert!(rig.view(&id).running.is_some());
    rig.c().set_enabled(&id, false, &Env::default()).unwrap();
    assert!(job.cancel.load(Ordering::SeqCst), "in flight is told to stop");
    rig.c().finish(&job, Outcome::Cancelled, now);
    rig.settle();
    assert_eq!(fake.calls(), 0, "the queue went with it");
    assert_eq!(rig.view(&id).pending, 0);
    assert!(rig.c().watch_set().is_empty());
}

#[test]
fn the_op_rows_cancel_stops_the_run_for_good() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    for n in ["a", "b", "c"] {
        std::fs::write(d.join(format!("{n}.png")), b"1").unwrap();
    }
    rig.c().mark_dirty(&id);
    rig.tick(0);
    rig.now.fetch_add(filter::STABLE_MS, Ordering::SeqCst);
    rig.c().mark_dirty(&id);
    let now = rig.at();
    let scans = rig.c().due_scans(now);
    for s in scans {
        let res = scan(Path::new(&s.folder), s.subfolders, &s.spec, &s.own, &via, &|| {});
        rig.c().apply_scan(&s.rule_id, res, now);
    }
    let job = rig.c().next_job(now).expect("a job");
    rig.c().began(&job, now);
    rig.c().cancel_rule(&id);
    rig.c().finish(&job, Outcome::Cancelled, now);
    rig.settle();
    assert_eq!(fake.calls(), 0);
    assert!(rig.c().take_events().iter().any(|e| e.cancelled));
    assert!(rig.view(&id).enabled, "new files still count");
}

#[test]
fn a_vanished_folder_is_quiet_and_resumes_when_back() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    std::fs::write(d.join("old.png"), b"1").unwrap();
    rig.settle();
    assert_eq!(fake.calls(), 1);
    std::fs::rename(&d, t.0.join("away")).unwrap();
    rig.settle();
    let v = rig.view(&id);
    assert_eq!(v.unavailable.as_deref(), Some("missing"));
    assert_eq!((v.failed, v.failures.len(), v.enabled), (0, 0, true), "not a stream of errors");
    let scans_while_gone = (0..10).map(|_| { rig.now.fetch_add(1000, Ordering::SeqCst); let now = rig.at(); rig.c().due_scans(now).len() }).sum::<usize>();
    assert!(scans_while_gone <= 1, "and no hammering: {scans_while_gone}");
    assert!(rig.c().watch_set().is_empty());
    std::fs::rename(t.0.join("away"), &d).unwrap();
    std::fs::write(d.join("new.png"), b"2").unwrap();
    rig.settle();
    assert_eq!(rig.view(&id).unavailable, None);
    assert_eq!(fake.calls(), 2, "the old file wasn't done again, the new one was");
}

#[test]
fn a_missing_output_folder_holds_files_without_using_up_attempts() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    std::fs::rename(&out, t.0.join("gone")).unwrap();
    std::fs::write(d.join("a.png"), b"1").unwrap();
    rig.settle();
    let v = rig.view(&id);
    assert_eq!((v.unavailable.as_deref(), v.failed, fake.calls()), (Some("outputMissing"), 0, 0));
    std::fs::rename(t.0.join("gone"), &out).unwrap();
    rig.settle();
    assert_eq!(fake.calls(), 1);
    assert_eq!(names(&out), ["a.webp"]);
}

#[test]
fn pause_all_holds_everything_and_is_remembered() {
    let t = Tmp::new();
    let (cfg, d, out) = (t.dir("cfg"), t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(Some(cfg.clone()), vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    rig.c().set_paused(true);
    std::fs::write(d.join("a.png"), b"1").unwrap();
    rig.settle();
    assert_eq!(fake.calls(), 0);
    assert!(rig.c().watch_set().is_empty());
    drop(rig);
    let rig = Rig::new(Some(cfg), vec![recipe("r", "webp", Some(&out))], &fake);
    assert!(rig.c().paused_all);
    rig.c().set_paused(false);
    rig.settle();
    assert_eq!(fake.calls(), 0, "paused across a restart: it is part of the baseline");
    assert_eq!(rig.view(&id).backlog, 1);
}

#[test]
fn stats_and_a_summary_per_batch() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    for n in ["a", "b", "c"] {
        std::fs::write(d.join(format!("{n}.png")), b"1").unwrap();
    }
    for _ in 0..10 {
        rig.c().mark_dirty(&id);
        rig.tick(1000);
    }
    let v = rig.view(&id);
    assert_eq!((v.done, v.failed), (3, 0));
    assert!(v.last_run.is_some());
    let ev = rig.c().take_events();
    assert_eq!(ev.len(), 1, "one report for the three");
    assert_eq!(ev[0].pairs.len(), 3);
}

// --- real engine

/// The whole path with the real encoder, when libvips is here. A file dropped next to itself must
/// yield one output, untouched source, no second generation.
#[test]
fn real_vips_run_if_available() {
    let has_vips = std::process::Command::new("vips").arg("--version").output().is_ok_and(|o| o.status.success());
    if !has_vips {
        eprintln!("skipped: no vips");
        return;
    }
    let t = Tmp::new();
    let d = t.dir("in");
    let mut c = Core::load(None);
    c.sync_recipes(vec![recipe("r", "webp", None)], &Env::default());
    c.add(d.to_string_lossy().into_owned(), "r".into(), false, "a".into(), &Env::default()).unwrap();
    c.set_enabled("a", true, &Env::default()).unwrap();
    let core = Mutex::new(c);
    let real: Box<ViaFn> = Box::new(|e, f| crate::convert::routes::detected(|a| crate::convert::routes::route_or_same(e, f, a)));
    let enc: Box<Encoder> = Box::new(|s, o, k| crate::convert::dispatch::convert_one(s, o, 64, k, &|_| {}));
    let mut now = 1_000_000u64;
    let drive = |now: &mut u64| {
        for _ in 0..6 {
            *now += 31_000;
            let scans = core.lock().unwrap().due_scans(*now);
            for s in scans {
                let res = scan(Path::new(&s.folder), s.subfolders, &s.spec, &s.own, &*real, &|| {});
                core.lock().unwrap().apply_scan(&s.rule_id, res, *now);
            }
            let job = core.lock().unwrap().next_job(*now);
            if let Some(j) = job {
                let at = *now;
                run_job(&core, j, &*enc, &*real, &to_scratch, &move || at);
            }
        }
    };
    drive(&mut now);
    let src = d.join("pic.png");
    assert!(std::process::Command::new("vips").arg("black").arg(&src).args(["16", "16"]).status().unwrap().success());
    let before = std::fs::read(&src).unwrap();
    drive(&mut now);
    drive(&mut now);
    assert_eq!(names(&d), ["pic.png", "pic.webp"]);
    assert_eq!(std::fs::read(&src).unwrap(), before);
    let v = core.lock().unwrap().overview();
    assert_eq!((v.rules[0].done, v.rules[0].failed), (1, 0));

    // The format checks against what a real encoder writes, whole and cut.
    use super::complete::{check, Verdict::*};
    for ext in ["jpg", "png", "gif", "webp", "tiff", "avif", "heic"] {
        let f = t.0.join(format!("real.{ext}"));
        let made = std::process::Command::new("vips").arg("black").arg(&f).args(["64", "64"]).output().is_ok_and(|o| o.status.success());
        if !made {
            eprintln!("skipped {ext}: this vips can't write it");
            continue;
        }
        let e = filter::fold_ext(ext);
        assert_eq!(check(&f, &e, &|| {}), Complete, "{ext} whole");
        let bytes = std::fs::read(&f).unwrap();
        std::fs::write(&f, &bytes[..bytes.len() * 2 / 3]).unwrap();
        assert_eq!(check(&f, &e, &|| {}), Incomplete, "{ext} cut");
    }
}

// --- QA regressions

fn write_png(p: &Path, chunks: usize) {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let chunk = |t: &[u8], d: &[u8], b: &mut Vec<u8>| {
        b.extend((d.len() as u32).to_be_bytes());
        b.extend(t);
        b.extend(d);
        b.extend([0u8; 4]);
    };
    for _ in 0..chunks {
        chunk(b"IDAT", &[7u8; 40], &mut b);
    }
    chunk(b"IEND", &[], &mut b);
    std::fs::write(p, b).unwrap();
}

#[test]
fn format_checks_tell_cut_from_whole() {
    use super::complete::{check, Verdict::*};
    let t = Tmp::new();
    let f = t.0.join("f");
    let verdict = |bytes: &[u8], ext: &str| {
        std::fs::write(&f, bytes).unwrap();
        check(&f, ext, &|| {})
    };
    // JPEG with a thumbnail's EOI inside APP1, entropy data, and data after the end.
    let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x06, 0xFF, 0xD9, 0, 0, 0xFF, 0xDA, 0x00, 0x02, 1, 2, 0xFF, 0x00, 3];
    assert_eq!(verdict(&jpg, "jpg"), Incomplete, "the thumbnail's marker doesn't count");
    jpg.extend([0xFF, 0xD9]);
    assert_eq!(verdict(&jpg, "jpg"), Complete);
    jpg.extend(b"appended motion photo");
    assert_eq!(verdict(&jpg, "jpg"), Complete);
    assert_eq!(verdict(&[0xFF], "jpg"), Incomplete);
    assert_eq!(verdict(b"not a jpeg", "jpg"), Unknown, "left to the engine to report");

    write_png(&f, 3);
    let whole = std::fs::read(&f).unwrap();
    assert_eq!(verdict(&whole, "png"), Complete);
    for cut in [4, 8, 30, whole.len() - 1] {
        assert_eq!(verdict(&whole[..cut], "png"), Incomplete, "cut at {cut}");
    }

    let gif = [b"GIF89a".as_slice(), &[1, 0, 1, 0, 0, 0, 0], &[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 1, 9, 0], &[0x3B]].concat();
    assert_eq!(verdict(&gif, "gif"), Complete);
    assert_eq!(verdict(&gif[..gif.len() - 1], "gif"), Incomplete);

    let mut webp = b"RIFF".to_vec();
    webp.extend(12u32.to_le_bytes());
    webp.extend(b"WEBPVP8 ");
    assert_eq!(verdict(&webp, "webp"), Incomplete);
    webp.extend([0; 4]);
    assert_eq!(verdict(&webp, "webp"), Complete);

    assert_eq!(verdict(b"%PDF-1.7 body", "pdf"), Incomplete);
    assert_eq!(verdict(b"%PDF-1.7 body\n%%EOF\n", "pdf"), Complete);
    assert_eq!(verdict(b"PK\x03\x04 data", "docx"), Incomplete);
    assert_eq!(verdict(b"PK\x03\x04 data PK\x05\x06", "docx"), Complete);

    let bx = |t: &[u8], n: u32| [n.to_be_bytes().as_slice(), t, &vec![0u8; n as usize - 8]].concat();
    let mp4 = [bx(b"ftyp", 16), bx(b"mdat", 32), bx(b"moov", 16)].concat();
    assert_eq!(verdict(&mp4, "mp4"), Complete);
    assert_eq!(verdict(&mp4[..40], "mp4"), Incomplete);
    assert_eq!(verdict(&mp4[..48], "mp4"), Incomplete, "boxes fit, the index isn't there yet");

    assert_eq!(verdict(b"anything", "mp3"), Unknown);
    assert!(!super::complete::checkable("mp3") && super::complete::checkable("png"));
}

#[test]
fn b1_a_paused_writer_keeps_the_file_waiting_and_the_finished_file_converts_once() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    let f = d.join("slow.png");
    write_png(&f, 4);
    let whole = std::fs::read(&f).unwrap();
    std::fs::write(&f, &whole[..whole.len() / 3]).unwrap();
    rig.settle();
    rig.settle();
    let v = rig.view(&id);
    assert_eq!((fake.calls(), v.failed, v.incomplete, v.pending), (0, 0, 1, 0), "waiting, not failed, and not busy");
    std::fs::write(&f, &whole).unwrap();
    rig.settle();
    assert_eq!(fake.calls(), 1);
    assert_eq!(names(&out), ["slow.webp"]);
    assert_eq!(rig.view(&id).incomplete, 0);
}

/// Encodes in one size, then the source grows, as with a format no check can see through.
#[test]
fn b1_an_output_from_an_unfinished_source_is_replaced_not_suffixed() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    let go = |ms| {
        for _ in 0..4 {
            rig.c().mark_dirty(&id);
            rig.tick(ms);
        }
    };
    go(1000);
    let f = d.join("a.gif");
    std::fs::write(&f, b"junk1").unwrap();
    go(2000);
    assert_eq!(names(&out), ["a.webp"]);
    std::fs::write(&f, b"junk1 and the rest").unwrap();
    go(2000);
    assert_eq!(fake.calls(), 2);
    assert_eq!(names(&out), ["a.webp"], "no suffixed twin");
    assert_eq!(std::fs::read(out.join("a.webp")).unwrap(), b"junk1 and the rest");

    // But an output someone has touched is theirs.
    let g = d.join("b.gif");
    std::fs::write(&g, b"junk1").unwrap();
    go(2000);
    std::fs::write(out.join("b.webp"), b"edited by hand").unwrap();
    std::fs::write(&g, b"junk1 and the rest").unwrap();
    go(2000);
    assert_eq!(names(&out), ["a.webp", "b (converted).webp", "b.webp"]);
    assert_eq!(std::fs::read(out.join("b.webp")).unwrap(), b"edited by hand");
}

#[test]
fn b2_a_full_ledger_never_reconverts_or_takes_an_output_for_a_source() {
    let t = Tmp::new();
    let (cfg, d) = (t.dir("cfg"), t.dir("in"));
    let fake = Fake::new();
    let make = |recipes: Vec<Recipe>| {
        let mut c = Core::load_capped(cfg.clone(), 5);
        c.sync_recipes(recipes, &Env::default());
        Rig { core: Mutex::new(c), now: Arc::new(AtomicU64::new(1_000_000)), enc: fake.encoder() }
    };
    let id;
    {
        let rig = make(vec![recipe("r", "webp", None), recipe("back", "png", None)]);
        id = rig.add_on(&d, "r", false);
        rig.settle();
        std::fs::write(d.join("seed.webp"), b"s").unwrap();
        for i in 0..20 {
            std::fs::write(d.join(format!("f{i}.png")), b"x").unwrap();
            for _ in 0..3 {
                rig.c().mark_dirty(&id);
                rig.tick(2000);
            }
        }
        assert_eq!(fake.calls(), 20);
        // Well past the bound, in session: one more file is one more conversion.
        std::fs::write(d.join("last.png"), b"x").unwrap();
        rig.settle();
        assert_eq!(fake.calls(), 21, "evicted entries didn't cascade");
        assert!(rig.c().overview().ledger_full);
    }
    let rig = make(vec![recipe("r", "webp", None), recipe("back", "png", None)]);
    rig.settle();
    std::fs::write(d.join("new.png"), b"x").unwrap();
    rig.settle();
    assert_eq!(fake.calls(), 22, "after a restart too");
    assert_eq!(rig.view(&id).backlog, 0, "full ledger: nothing is offered");
    rig.c().process_existing(&id, 1);
    rig.settle();
    assert_eq!(fake.calls(), 22, "and the opt-in fails safe");
    assert!(!names(&d).iter().any(|n| n.contains("converted")));
}

#[test]
fn b2_the_ledger_makes_room_from_files_that_are_gone() {
    let t = Tmp::new();
    let mut l = Ledger::with_cap(Some(t.0.join("l.jsonl")), 10);
    for i in 0..10 {
        l.mark_handled(&ledger::key("r", &format!("/p/{i}"), 1, 1), "");
    }
    assert!(l.wants_sweep() && !l.lossy());
    l.drop_gone(&|p: &str| p == "/p/0");
    assert!(l.handled(&ledger::key("r", "/p/0", 1, 1)) && !l.handled(&ledger::key("r", "/p/1", 1, 1)));
    l.mark_produced("/out", "");
    for i in 0..8 {
        l.mark_handled(&ledger::key("r", &format!("/q/{i}"), 1, 1), "");
    }
    assert!(!l.lossy(), "room was made, nothing live lost");
    assert!(Ledger::with_cap(Some(t.0.join("l.jsonl")), 10).handled(&ledger::key("r", "/p/0", 1, 1)));
}

/// QA's scenario: opposing rules, one folder, two processes.
#[test]
fn b3_a_second_instance_runs_nothing() {
    let t = Tmp::new();
    let (cfg, d) = (t.dir("cfg"), t.dir("in"));
    let recipes = vec![recipe("w", "webp", None), recipe("p", "png", None)];
    let (f1, f2) = (Fake::new(), Fake::new());
    let a = Rig::new(Some(cfg.clone()), recipes.clone(), &f1);
    let wid = a.add_on(&d, "w", false);
    a.settle();
    let b = Rig::new(Some(cfg.clone()), recipes.clone(), &f2);
    assert!(b.c().overview().standby, "and says so");
    let refused = b.c().add(d.to_string_lossy().into_owned(), "p".into(), false, "x".into(), &Env::default());
    assert_eq!(refused, Err("otherInstance".to_string()));
    assert_eq!(b.c().set_enabled(&wid, false, &Env::default()), Err("otherInstance".to_string()));
    std::fs::write(d.join("a.png"), b"1").unwrap();
    for _ in 0..40 {
        a.c().mark_dirty(&wid);
        a.tick(2000);
        b.c().mark_dirty(&wid);
        b.tick(2000);
    }
    assert_eq!((f1.calls(), f2.calls()), (1, 0));
    assert_eq!(names(&d), ["a.png", "a.webp"]);
    assert!(a.view(&wid).enabled, "the standby wrote nothing over the owner's rules");
    drop(a); // released however the first one ends
    assert!(b.c().try_take_over(&Env::default()));
    assert!(!b.c().overview().standby);
    b.settle();
    std::fs::write(d.join("c.png"), b"1").unwrap();
    b.settle();
    assert_eq!(f2.calls(), 1, "and it carries on, without redoing the first");
}

#[test]
fn n3_no_rules_leave_no_trace() {
    let t = Tmp::new();
    let cfg = t.dir("cfg");
    let mut c = Core::load(Some(cfg.clone()));
    c.sync_recipes(vec![recipe("r", "webp", None)], &Env::default());
    c.save();
    assert!(names(&cfg).is_empty(), "{:?}", names(&cfg));
    assert!(!c.overview().standby);
}

#[cfg(unix)]
#[test]
fn n1_a_scan_problem_or_an_empty_mountpoint_does_not_lose_the_baseline() {
    use std::os::unix::fs::PermissionsExt;
    let t = Tmp::new();
    let (d, sub, out) = (t.dir("in"), t.dir("in/sub"), t.dir("out"));
    std::fs::write(d.join("top.png"), b"1").unwrap();
    std::fs::write(sub.join("deep.png"), b"1").unwrap();
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", true);
    rig.settle();
    assert_eq!(rig.view(&id).backlog, 2);
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();
    rig.settle();
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
    rig.settle();
    assert_eq!(fake.calls(), 0, "unreadable subfolder for a while");
    // Replaced by an empty folder, then restored.
    std::fs::rename(&d, t.0.join("real")).unwrap();
    std::fs::create_dir(&d).unwrap();
    rig.settle();
    assert_eq!((rig.view(&id).unavailable.clone(), rig.view(&id).backlog), (None, 0));
    std::fs::remove_dir(&d).unwrap();
    std::fs::rename(t.0.join("real"), &d).unwrap();
    rig.settle();
    assert_eq!(fake.calls(), 0, "nothing re-converted without an opt-in");
    assert_eq!(rig.view(&id).backlog, 2);
}

#[cfg(unix)]
#[test]
fn n2_renames_are_not_new_files() {
    let t = Tmp::new();
    let (cfg, d, sub, out) = (t.dir("cfg"), t.dir("in"), t.dir("in/sub"), t.dir("out"));
    std::fs::write(d.join("old.png"), b"1").unwrap();
    let fake = Fake::new();
    let recipes = vec![recipe("r", "webp", Some(&out))];
    let id;
    {
        let rig = Rig::new(Some(cfg.clone()), recipes.clone(), &fake);
        id = rig.add_on(&d, "r", true);
        rig.settle();
        std::fs::write(sub.join("a.png"), b"2").unwrap();
        rig.settle();
        assert_eq!(fake.calls(), 1);
        std::fs::rename(sub.join("a.png"), sub.join("b.png")).unwrap();
        std::fs::rename(d.join("old.png"), d.join("older.png")).unwrap();
        rig.settle();
        std::fs::rename(&sub, d.join("moved")).unwrap();
        rig.settle();
        assert_eq!(fake.calls(), 1);
        assert_eq!(rig.view(&id).backlog, 1, "a renamed untouched file is still just on offer");
    }
    let rig = Rig::new(Some(cfg), recipes, &fake);
    rig.settle();
    assert_eq!(rig.view(&id).backlog, 1, "the done one isn't offered after a restart either");
}

#[test]
fn n4_n5_nothing_rescans_every_second_for_a_placeholder_or_a_held_rule() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    let count = |rig: &Rig| {
        (0..20).map(|_| { rig.now.fetch_add(1000, Ordering::SeqCst); let now = rig.at(); let s = rig.c().due_scans(now); let n = s.len(); for x in s { let r = scan(Path::new(&x.folder), x.subfolders, &x.spec, &x.own, &via, &|| {}); rig.c().apply_scan(&x.rule_id, r, now); } n }).sum::<usize>()
    };
    std::fs::write(d.join("empty.png"), b"").unwrap();
    rig.c().mark_dirty(&id);
    assert!(count(&rig) <= 2);
    assert_eq!(rig.view(&id).pending, 0, "a placeholder isn't pending");
    for i in 0..MASS_ASK {
        std::fs::write(d.join(format!("f{i}.png")), b"x").unwrap();
    }
    rig.c().mark_dirty(&id);
    rig.tick(0);
    assert_eq!(rig.view(&id).hold, Some(Hold::Mass));
    assert!(count(&rig) <= 1, "held: every 30 s at most");
    std::fs::write(d.join("empty.png"), b"now full").unwrap();
}

#[test]
fn n6_real_rules_on_volumes_and_library() {
    let env = rules::RealEnv;
    if cfg!(target_os = "macos") {
        assert!(env.protected(Path::new("/Volumes")));
        assert!(!env.protected(Path::new("/Volumes/Stick/in")));
        let home = std::fs::canonicalize(dirs::home_dir().unwrap()).unwrap();
        assert!(env.protected(&home.join("Library/Caches/x")));
        assert!(!env.protected(&home.join("Downloads/in")));
    }
}

#[test]
fn n7_sweeps_only_our_exact_temps_of_dead_processes() {
    let t = Tmp::new();
    assert_eq!(filter::temp_owner_pid(".morfa-convert-123-4.webp"), Some(123));
    for n in [".morfa-convert-x-4.webp", ".morfa-convert-123.webp", ".morfa-convert-1-2-3.webp", "morfa-convert-1-2.webp", ".morfa-other-1-2.webp"] {
        assert_eq!(filter::temp_owner_pid(n), None, "{n}");
    }
    let d = t.dir("in");
    // A fresh file of a dead process stays: the age limit. Ours stays whatever its age.
    std::fs::write(d.join(".morfa-convert-999999-1.webp"), b"x").unwrap();
    assert_eq!(sweep_temps(&d), 0);
    assert_eq!(names(&d).len(), 1);
    let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let age = |n: &str| {
        let f = std::fs::File::options().write(true).create(true).truncate(false).open(d.join(n)).unwrap();
        f.set_modified(hour_ago).unwrap();
    };
    age(".morfa-convert-999999-1.webp");
    age(&format!(".morfa-convert-{}-1.webp", std::process::id()));
    age(".morfa-convert-note.webp");
    age("keep.webp");
    #[cfg(unix)]
    {
        assert_eq!(sweep_temps(&d), 1);
        assert_eq!(names(&d).len(), 3, "live owner and lookalikes stay: {:?}", names(&d));
    }
}

#[test]
fn n9_gate_agrees_with_the_ui_on_lossless_limits() {
    let mut o = opts("png", None);
    o.max_bytes = Some(2048);
    assert_eq!(filter::gate(&o, "jpg", Some(Via::Image)).max_bytes, None);
    let mut o = opts("webp", None);
    o.max_bytes = Some(2048);
    assert_eq!(filter::gate(&o, "jpg", Some(Via::Image)).max_bytes, Some(2048));
}

#[test]
fn n10_a_lost_ledger_does_not_offer_our_outputs() {
    let t = Tmp::new();
    let (a, b) = (t.dir("a"), t.dir("b"));
    std::fs::write(b.join("x.webp"), b"1").unwrap();
    std::fs::write(b.join("y (converted).jpg"), b"1").unwrap();
    std::fs::write(b.join("z (converted 3).jpg"), b"1").unwrap();
    std::fs::write(b.join("mine.jpg"), b"1").unwrap();
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("to-b", "webp", Some(&b)), recipe("r", "png", Some(&a))], &fake);
    let _feeds = rig.add_on(&a, "to-b", false);
    let id = rig.add_on(&b, "r", false);
    rig.settle();
    assert_eq!(rig.view(&id).backlog, 1, "only mine.jpg");
}

#[test]
fn n12_a_big_queue_rescan_is_linear() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    rig.settle();
    let files: Vec<Seen> = (0..50_000).map(|i| Seen { path: format!("/nowhere/{i}.png"), size: 1, mtime: 1, id: String::new(), ours: false }).collect();
    rig.c().apply_scan(&id, Ok(ScanOut { files: files.clone(), ..Default::default() }), 1);
    rig.c().resolve(&id, true);
    rig.c().apply_scan(&id, Ok(ScanOut { files: files.clone(), ..Default::default() }), 10_000_000);
    let started = std::time::Instant::now();
    rig.c().apply_scan(&id, Ok(ScanOut { files, ..Default::default() }), 10_000_001);
    assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
}

// --- second QA round

/// Two windows that both started with no rules. The first adds one, works, and exits.
#[test]
fn a_window_that_starts_later_keeps_the_rule_and_record_of_the_one_that_left() {
    let t = Tmp::new();
    let (cfg, a_dir, b_dir, out) = (t.dir("cfg"), t.dir("a"), t.dir("b"), t.dir("out"));
    let recipes = vec![recipe("r", "webp", Some(&out))];
    let (f1, f2) = (Fake::new(), Fake::new());
    let a = Rig::new(Some(cfg.clone()), recipes.clone(), &f1);
    let b = Rig::new(Some(cfg.clone()), recipes.clone(), &f2);
    let aid = a.add_on(&a_dir, "r", false);
    a.settle();
    std::fs::write(a_dir.join("x.png"), b"1").unwrap();
    a.settle();
    assert_eq!(a.view(&aid).done, 1);
    drop(a);
    let bid = b.add_on(&b_dir, "r", false);
    let saved = std::fs::read_to_string(cfg.join("autoconvert.json")).unwrap();
    assert!(saved.contains(&aid) && saved.contains(&bid), "both rules stored");
    let v = b.view(&aid);
    assert_eq!((v.done, v.enabled), (1, true), "with its stats, and still on");
    b.settle();
    b.c().process_existing(&aid, 1);
    b.settle();
    assert_eq!(f2.calls(), 0, "the first window's record came along");
    assert_eq!(b.view(&aid).backlog, 0);
}

#[test]
fn a_standby_writes_nothing() {
    let t = Tmp::new();
    let (cfg, d) = (t.dir("cfg"), t.dir("in"));
    let fake = Fake::new();
    let a = Rig::new(Some(cfg.clone()), vec![recipe("r", "webp", None)], &fake);
    a.add_on(&d, "r", false);
    // Over the compaction limit for the cap below, with a torn tail.
    let ledger = cfg.join("autoconvert-ledger.jsonl");
    let text = format!("{}[\"k\",\"torn", "[\"k\",\"same\",\"\"]\n".repeat(1100));
    std::fs::write(&ledger, &text).unwrap();
    let before = std::fs::read_to_string(cfg.join("autoconvert.json")).unwrap();
    let mut b = Core::load_capped(cfg.clone(), 5);
    b.sync_recipes(vec![], &Env::default());
    b.save();
    assert!(b.overview().standby);
    assert_eq!(std::fs::read_to_string(&ledger).unwrap(), text);
    assert_eq!(std::fs::read_to_string(cfg.join("autoconvert.json")).unwrap(), before);
    // A file it can't read isn't its to move either.
    std::fs::write(cfg.join("autoconvert.json"), "{ broken").unwrap();
    let _c = Core::load(Some(cfg.clone()));
    assert!(cfg.join("autoconvert.json").exists() && !cfg.join("autoconvert.json.bad").exists());
}

/// A short-tick rig around one rule, on a `.gif` the fake encoder takes.
fn short_ticks(rig: &Rig, id: &str) {
    for _ in 0..4 {
        rig.c().mark_dirty(id);
        rig.tick(2000);
    }
}

#[test]
fn an_unrelated_bigger_file_of_the_same_name_keeps_both_outputs() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    short_ticks(&rig, &id);
    let f = d.join("a.gif");
    std::fs::write(&f, b"first").unwrap();
    short_ticks(&rig, &id);
    // Deleted, then another file dropped under the name. The first is moved aside, not deleted, so the new file gets another inode.
    let keep = t.0.join("first-file");
    std::fs::rename(&f, &keep).unwrap();
    std::fs::write(&f, b"first and unrelated").unwrap();
    short_ticks(&rig, &id);
    assert_eq!(fake.calls(), 2);
    assert_eq!(names(&out), ["a (converted).webp", "a.webp"]);
    assert_eq!(std::fs::read(out.join("a.webp")).unwrap(), b"first");
}

#[test]
fn an_edit_in_place_that_grows_a_file_keeps_both_outputs() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    short_ticks(&rig, &id);
    let f = d.join("a.gif");
    std::fs::write(&f, b"first").unwrap();
    short_ticks(&rig, &id);
    std::fs::write(&f, b"FIRST, redone and longer").unwrap(); // same inode, not a prefix
    short_ticks(&rig, &id);
    assert_eq!(names(&out), ["a (converted).webp", "a.webp"]);
    assert_eq!(std::fs::read(out.join("a.webp")).unwrap(), b"first");
}

#[test]
fn an_encoder_writing_nothing_never_costs_the_earlier_output() {
    let t = Tmp::new();
    let (d, out) = (t.dir("in"), t.dir("out"));
    let fake = Fake::new();
    let empty = Arc::new(AtomicUsize::new(0));
    let mut rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let (inner, flag) = (fake.encoder(), empty.clone());
    rig.enc = Box::new(move |s, o, k| if flag.load(Ordering::SeqCst) == 1 { Ok(vec![]) } else { inner(s, o, k) });
    let id = rig.add_on(&d, "r", false);
    short_ticks(&rig, &id);
    let f = d.join("a.gif");
    std::fs::write(&f, b"first").unwrap();
    short_ticks(&rig, &id);
    empty.store(1, Ordering::SeqCst);
    std::fs::write(&f, b"first and the rest").unwrap();
    short_ticks(&rig, &id);
    assert_eq!(names(&out), ["a.webp"]);
    assert_eq!(std::fs::read(out.join("a.webp")).unwrap(), b"first");
}

#[test]
fn a_recipe_change_never_replaces_across_format_or_folder() {
    let t = Tmp::new();
    let (d, out, other) = (t.dir("in"), t.dir("out"), t.dir("other"));
    let fake = Fake::new();
    let rig = Rig::new(None, vec![recipe("r", "webp", Some(&out))], &fake);
    let id = rig.add_on(&d, "r", false);
    short_ticks(&rig, &id);
    let f = d.join("a.gif");
    std::fs::write(&f, b"one").unwrap();
    short_ticks(&rig, &id);
    rig.c().sync_recipes(vec![recipe("r", "png", Some(&out))], &Env::default());
    std::fs::write(&f, b"one two").unwrap();
    short_ticks(&rig, &id);
    assert_eq!(names(&out), ["a.png", "a.webp"], "a.webp doesn't end up holding a png");
    assert_eq!(std::fs::read(out.join("a.webp")).unwrap(), b"one");
    rig.c().sync_recipes(vec![recipe("r", "png", Some(&other))], &Env::default());
    std::fs::write(&f, b"one two three").unwrap();
    short_ticks(&rig, &id);
    assert_eq!(names(&other), ["a.png"]);
    assert_eq!(names(&out), ["a.png", "a.webp"], "the old folder keeps its file");
}

#[test]
fn replacing_needs_a_written_file_of_the_same_folder_and_extension() {
    let t = Tmp::new();
    let (x, y) = (t.dir("x"), t.dir("y"));
    let stat = |p: PathBuf| {
        std::fs::write(&p, b"data").unwrap();
        let m = p.symlink_metadata().unwrap();
        #[cfg(unix)]
        let id = { use std::os::unix::fs::MetadataExt; format!("{}:{}", m.dev(), m.ino()) };
        #[cfg(not(unix))]
        let id = String::new();
        Output { path: p.to_string_lossy().into_owned(), id, size: m.len(), mtime: m.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64 }
    };
    let old = stat(x.join("a.webp"));
    let never = |_: &Path| -> bool { panic!("must not discard") };
    let other_ext = stat(x.join("a (converted).png"));
    assert_eq!(replace_damaged(vec![old.clone()], vec![other_ext], &never).len(), 1);
    let other_dir = stat(y.join("a.webp"));
    assert_eq!(replace_damaged(vec![old.clone()], vec![other_dir], &never).len(), 1);
    assert!(replace_damaged(vec![old.clone()], vec![], &never).is_empty());
    let new = stat(x.join("a (converted).webp"));
    std::fs::remove_file(&new.path).unwrap();
    assert_eq!(replace_damaged(vec![old.clone()], vec![new], &never).len(), 1, "the replacement isn't there");
    assert!(Path::new(&old.path).exists());
}

// --- third QA round

#[test]
fn f1_rules_that_cannot_be_read_are_never_saved_over() {
    let t = Tmp::new();
    let (cfg, d) = (t.dir("cfg"), t.dir("in"));
    let json = cfg.join("autoconvert.json");
    // Bad content: set aside, kept.
    std::fs::write(&json, [0xFF, 0xFE, b'{', 0x80]).unwrap();
    let c = Core::load(Some(cfg.clone()));
    assert!(c.rules.is_empty());
    assert_eq!(std::fs::read(cfg.join("autoconvert.json.bad")).unwrap(), [0xFF, 0xFE, b'{', 0x80]);
    drop(c);

    // Can't be read at all (a folder gives a read error other than "not found").
    std::fs::create_dir(&json).unwrap();
    let mut c = Core::load(Some(cfg.clone()));
    c.sync_recipes(vec![recipe("r", "webp", None)], &Env::default());
    c.save();
    assert!(json.is_dir(), "nothing written over it");
    assert!(c.overview().store_error && !c.overview().standby && c.wants_driver());
    let refused = c.add(d.to_string_lossy().into_owned(), "r".into(), false, "new".into(), &Env::default());
    assert_eq!(refused, Err("storeUnreadable".to_string()));
    assert!(!c.try_take_over(&Env::default()));
    assert!(json.is_dir());

    // It comes back: its rules are there, untouched.
    std::fs::remove_dir(&json).unwrap();
    let rule = r#"{"pausedAll":false,"rules":[{"id":"old","folder":"/nowhere","recipeId":"r","recipeName":"r"}]}"#;
    std::fs::write(&json, rule).unwrap();
    assert!(c.try_take_over(&Env::default()));
    assert_eq!(c.rules.len(), 1);
    assert!(!c.overview().store_error);
}

/// B started with no rules, A added one and left, B adds the same folder and is refused.
#[test]
fn f2_a_refused_add_still_takes_over_what_it_found() {
    let t = Tmp::new();
    let (cfg, d) = (t.dir("cfg"), t.dir("in"));
    let recipes = vec![recipe("r", "webp", None)];
    let fake = Fake::new();
    let a = Rig::new(Some(cfg.clone()), recipes.clone(), &fake);
    let b = Rig::new(Some(cfg.clone()), recipes, &fake);
    assert!(!b.c().wants_driver(), "no rules, no thread");
    let id = a.add_on(&d, "r", false);
    drop(a);
    let res = b.c().add(d.to_string_lossy().into_owned(), "r".into(), false, "dup".into(), &Env::default());
    assert_eq!(res, Err("duplicate".to_string()));
    assert!(b.c().wants_driver(), "the command starts the driver from this");
    assert!(b.view(&id).enabled, "the view it returns shows the rule");
    b.settle();
    std::fs::write(d.join("a.png"), b"1").unwrap();
    b.settle();
    assert_eq!(fake.calls(), 1, "and the rule runs here now");
}

#[test]
fn f4_without_hard_links_the_output_still_reaches_its_name_and_overwrites_nothing() {
    let t = Tmp::new();
    let d = t.dir("x");
    let no_links = |_: &Path, _: &Path| -> std::io::Result<()> { Err(std::io::Error::from(std::io::ErrorKind::Unsupported)) };
    let (from, to) = (d.join("a (converted).webp"), d.join("a.webp"));
    std::fs::write(&from, b"good").unwrap();
    let placed = place_exclusive(&from, &to, &no_links).unwrap();
    assert_eq!(std::fs::read(&to).unwrap(), b"good");
    assert!(!from.exists());
    assert_eq!((placed.path.as_str(), placed.size), (to.to_str().unwrap(), 4));

    // Someone took the name in between: theirs stays, ours stays.
    std::fs::write(&from, b"good").unwrap();
    std::fs::write(&to, b"theirs").unwrap();
    assert!(place_exclusive(&from, &to, &no_links).is_none());
    assert_eq!(std::fs::read(&to).unwrap(), b"theirs");
    assert_eq!(std::fs::read(&from).unwrap(), b"good");
    let taken = |a: &Path, b: &Path| std::fs::hard_link(a, b);
    assert!(place_exclusive(&from, &to, &taken).is_none());
    assert_eq!(std::fs::read(&to).unwrap(), b"theirs");
}
