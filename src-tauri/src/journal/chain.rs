//! Follows paths through the journal: what it knows, not what's necessarily on disk.
use super::store::{Entry, Kind};
use serde::Serialize;
use std::path::Path;

const MAX_CHAINS: usize = 100;

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum End {
    Located,
    Trash,
    Deleted,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Hop {
    pub entry: u64,
    pub time: u64,
    pub kind: Kind,
    pub from: String,
    pub to: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Chain {
    pub start: String,
    pub latest: String,
    pub end: End,
    pub hops: Vec<Hop>,
    /// Filled in by the command: `latest` is on disk.
    pub exists: bool,
}

/// `path` after `from` moved to `to`: for `from` itself or anything inside it.
pub(crate) fn remap(path: &str, from: &str, to: &str) -> Option<String> {
    if from.is_empty() {
        return None;
    }
    if path == from {
        return Some(to.to_string());
    }
    let rest = Path::new(path).strip_prefix(from).ok()?;
    Some(Path::new(to).join(rest).to_string_lossy().into_owned())
}

fn base(p: &str) -> String {
    Path::new(p).file_name().map_or(String::new(), |n| n.to_string_lossy().to_lowercase())
}

enum Query {
    Path(String),
    Name(String),
}
impl Query {
    fn parse(q: &str) -> Self {
        let q = q.trim();
        if q.contains(['/', '\\']) {
            Query::Path(q.trim_end_matches(['/', '\\']).to_string())
        } else {
            Query::Name(q.to_lowercase())
        }
    }
    /// The (from, to) of the query's own item under this move, if it's about it.
    fn on_move(&self, from: &str, to: &str) -> Option<(String, String)> {
        match self {
            Query::Name(n) => (base(from).contains(n) || base(to).contains(n)).then(|| (from.to_string(), to.to_string())),
            Query::Path(q) => {
                if let Some(t) = remap(q, from, to) {
                    return Some((q.clone(), t));
                }
                remap(q, to, from).map(|f| (f, q.clone()))
            }
        }
    }
    fn on_end(&self, from: &str) -> Option<String> {
        match self {
            Query::Name(n) => base(from).contains(n).then(|| from.to_string()),
            Query::Path(q) => remap(q, from, from),
        }
    }
}

/// Every chain the query starts, newest change first. Whatever the entries are, each chain is one
/// pass through them, so it can't loop.
pub fn find(entries: &[Entry], query: &str) -> Vec<Chain> {
    let q = Query::parse(query);
    if matches!(&q, Query::Name(n) if n.is_empty()) || matches!(&q, Query::Path(p) if p.is_empty()) {
        return vec![];
    }
    let mut out: Vec<Chain> = vec![];
    for e in entries {
        if e.kind.moves() {
            // One entry's pairs are one change: a swap moves each file once, not there and back.
            let mut used = vec![false; e.pairs.len()];
            for c in out.iter_mut().filter(|c| c.end == End::Located) {
                let hit = e.pairs.iter().enumerate().filter(|(_, p)| p.done).find_map(|(i, p)| remap(&c.latest, &p.from, &p.to).map(|n| (i, n)));
                if let Some((i, next)) = hit {
                    used[i] = true;
                    c.hops.push(Hop { entry: e.id, time: e.time, kind: e.kind, from: std::mem::replace(&mut c.latest, next.clone()), to: next });
                }
            }
            for (_, pair) in e.pairs.iter().enumerate().filter(|(i, p)| p.done && !used[*i]) {
                if out.len() < MAX_CHAINS {
                    if let Some((from, to)) = q.on_move(&pair.from, &pair.to) {
                        out.push(Chain { start: from.clone(), latest: to.clone(), end: End::Located, hops: vec![Hop { entry: e.id, time: e.time, kind: e.kind, from, to }], exists: false });
                    }
                }
            }
            continue;
        }
        for pair in e.pairs.iter().filter(|p| p.done) {
            if e.kind.trashes() || e.kind == Kind::Delete {
                let end = if e.kind == Kind::Delete { End::Deleted } else { End::Trash };
                let mut hit = false;
                for c in out.iter_mut().filter(|c| c.end == End::Located) {
                    if remap(&c.latest, &pair.from, &pair.from).is_some() {
                        c.hops.push(Hop { entry: e.id, time: e.time, kind: e.kind, from: c.latest.clone(), to: String::new() });
                        c.end = end;
                        hit = true;
                    }
                }
                if !hit && out.len() < MAX_CHAINS {
                    if let Some(from) = q.on_end(&pair.from) {
                        out.push(Chain { start: from.clone(), latest: from.clone(), end, hops: vec![Hop { entry: e.id, time: e.time, kind: e.kind, from, to: String::new() }], exists: false });
                    }
                }
            } else if e.kind == Kind::Restore {
                for c in out.iter_mut().filter(|c| c.end == End::Trash && c.latest == pair.to) {
                    c.hops.push(Hop { entry: e.id, time: e.time, kind: e.kind, from: String::new(), to: pair.to.clone() });
                    c.end = End::Located;
                }
            }
        }
    }
    out.sort_by_key(|c| std::cmp::Reverse(c.hops.last().map_or(0, |h| h.entry)));
    out
}

/// Where `path` is after these entries, as far as they say.
pub fn follow<'a>(entries: impl Iterator<Item = &'a Entry>, path: &str) -> (String, End) {
    let mut cur = path.to_string();
    let mut end = End::Located;
    for e in entries {
        if e.kind.moves() {
            if end == End::Located {
                if let Some(next) = e.pairs.iter().filter(|p| p.done).find_map(|p| remap(&cur, &p.from, &p.to)) {
                    cur = next;
                }
            }
            continue;
        }
        for pair in e.pairs.iter().filter(|p| p.done) {
            if e.kind == Kind::Restore {
                if end == End::Trash && cur == pair.to {
                    end = End::Located;
                }
            } else if end != End::Located {
                continue;
            } else if (e.kind.trashes() || e.kind == Kind::Delete) && remap(&cur, &pair.from, &pair.from).is_some() {
                end = if e.kind == Kind::Delete { End::Deleted } else { End::Trash };
            }
        }
    }
    (cur, end)
}
