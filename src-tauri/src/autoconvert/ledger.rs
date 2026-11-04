//! What must survive a restart: which file versions a rule has dealt with, and which files are our
//! own outputs. An append-only log, bounded. The bound is safe by construction: every file present
//! at launch joins the baseline whether known here or not, so a forgotten entry can't start a
//! conversion. What it could still do is make an old file look offerable, so entries of files that
//! are gone go first (`drop_gone`), and once a live one has been forgotten the log says so (`lossy`)
//! and the opt-in stops being offered.
use std::{
    collections::{HashSet, VecDeque},
    io::Write,
    path::PathBuf,
};

pub const MAX_ENTRIES: usize = 50_000;

/// By path, and by file identity where there is one, so a rename doesn't make a file new.
#[derive(Clone)]
struct Item {
    key: String,
    id: String,
}

struct Bounded {
    order: VecDeque<Item>,
    set: HashSet<String>,
    cap: usize,
}

impl Bounded {
    fn new(cap: usize) -> Self {
        Bounded { order: VecDeque::new(), set: HashSet::new(), cap }
    }
    /// Whether it was new, and whether something had to be forgotten.
    fn add(&mut self, key: &str, id: &str) -> (bool, bool) {
        if self.set.contains(key) {
            return (false, false);
        }
        self.set.insert(key.to_string());
        if !id.is_empty() {
            self.set.insert(id.to_string());
        }
        self.order.push_back(Item { key: key.into(), id: id.into() });
        let mut evicted = false;
        while self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old.key);
                self.set.remove(&old.id);
                evicted = true;
            }
        }
        (true, evicted)
    }
    fn remove(&mut self, key: &str) {
        if self.set.contains(key) {
            let set = &mut self.set;
            self.order.retain(|x| {
                let hit = x.key == key;
                if hit {
                    set.remove(&x.key);
                    set.remove(&x.id);
                }
                !hit
            });
        }
    }
    fn retain_live(&mut self, path_of: impl Fn(&str) -> String, live: &dyn Fn(&str) -> bool) {
        let set = &mut self.set;
        self.order.retain(|x| {
            let keep = live(&path_of(&x.key));
            if !keep {
                set.remove(&x.key);
                set.remove(&x.id);
            }
            keep
        });
    }
}

pub struct Ledger {
    file: Option<PathBuf>,
    handled: Bounded,
    produced: Bounded,
    lines: usize,
    cap: usize,
    lossy: bool,
}

pub fn key(rule: &str, path: &str, size: u64, mtime: i64) -> String {
    format!("{rule}\0{path}\0{size}\0{mtime}")
}

fn path_in_key(k: &str) -> String {
    k.split('\0').nth(1).unwrap_or_default().to_string()
}

impl Ledger {
    pub fn open(file: Option<PathBuf>) -> Self {
        Self::with_cap(file, MAX_ENTRIES)
    }

    pub fn with_cap(file: Option<PathBuf>, cap: usize) -> Self {
        let mut l = Ledger { file, handled: Bounded::new(cap), produced: Bounded::new(cap), lines: 0, cap, lossy: false };
        let text = l.file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
        // A torn last line, from a crash mid-write, just doesn't parse.
        for line in text.lines() {
            let Ok((op, v, id)) = serde_json::from_str::<(String, String, String)>(line) else { continue };
            let evicted = match op.as_str() {
                "k" => l.handled.add(&v, &id).1,
                "p" => l.produced.add(&v, &id).1,
                "f" => {
                    l.handled.remove(&v);
                    false
                }
                "l" => true,
                _ => false,
            };
            l.lossy |= evicted;
            l.lines += 1;
        }
        // Finish a torn line, or the next record would be glued to it and lost as well.
        if !text.is_empty() && !text.ends_with('\n') {
            if let Some(f) = &l.file {
                let _ = std::fs::OpenOptions::new().append(true).open(f).and_then(|mut h| writeln!(h));
            }
        }
        l.compact_if_needed();
        l
    }

    fn append(&mut self, op: &str, v: &str, id: &str) {
        self.lines += 1;
        if let Some(f) = &self.file {
            if let Ok(line) = serde_json::to_string(&(op, v, id)) {
                let r = std::fs::OpenOptions::new().create(true).append(true).open(f).and_then(|mut h| writeln!(h, "{line}"));
                if let Err(e) = r {
                    eprintln!("autoconvert ledger: {e}");
                }
            }
        }
        self.compact_if_needed();
    }

    fn rewrite(&mut self) {
        self.lines = self.handled.order.len() + self.produced.order.len() + self.lossy as usize;
        let Some(f) = self.file.clone() else { return };
        let rows = self.handled.order.iter().map(|v| ("k", v)).chain(self.produced.order.iter().map(|v| ("p", v)));
        let mut text: String = if self.lossy { "[\"l\",\"\",\"\"]\n".to_string() } else { String::new() };
        text.extend(rows.filter_map(|(op, i)| serde_json::to_string(&(op, &i.key, &i.id)).ok()).map(|l| l + "\n"));
        let tmp = f.with_extension("tmp");
        if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &f)) {
            eprintln!("autoconvert ledger: {e}");
        }
    }

    fn compact_if_needed(&mut self) {
        if self.lines > 2 * self.cap + 1000 {
            self.rewrite();
        }
    }

    pub fn handled(&self, k: &str) -> bool {
        self.handled.set.contains(k)
    }
    pub fn mark_handled(&mut self, k: &str, id: &str) {
        let (new, evicted) = self.handled.add(k, id);
        self.lossy |= evicted;
        if new {
            self.append("k", k, id);
        }
        if evicted {
            self.append("l", "", "");
        }
    }
    pub fn forget(&mut self, k: &str) {
        self.handled.remove(k);
        self.append("f", k, "");
    }
    pub fn produced(&self, key: &str) -> bool {
        self.produced.set.contains(key)
    }
    pub fn mark_produced(&mut self, path: &str, id: &str) {
        let (new, evicted) = self.produced.add(path, id);
        self.lossy |= evicted;
        if new {
            self.append("p", path, id);
        }
        if evicted {
            self.append("l", "", "");
        }
    }
    /// A live entry was forgotten at some point: "already dealt with" can't be told any more.
    pub fn lossy(&self) -> bool {
        self.lossy
    }
    /// Close enough to the bound to make room while nothing has to be lost.
    pub fn wants_sweep(&self) -> bool {
        self.handled.order.len().max(self.produced.order.len()) * 10 >= self.cap * 9
    }
    pub fn paths(&self) -> Vec<String> {
        self.handled.order.iter().map(|i| path_in_key(&i.key)).chain(self.produced.order.iter().map(|i| i.key.clone())).collect()
    }
    /// Forgets files that no longer exist. `live` must answer true when it can't tell.
    pub fn drop_gone(&mut self, live: &dyn Fn(&str) -> bool) {
        self.handled.retain_live(path_in_key, live);
        self.produced.retain_live(str::to_string, live);
        self.rewrite();
    }
}
