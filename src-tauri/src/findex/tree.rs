//! The index: one flat node table plus two byte arenas. About 28 bytes a node plus its name.
//! Folded names sit in one `\0`-separated arena in node order, so a query is a single substring scan
//! and a hit maps back to its node by binary search. Original names are stored again only when they
//! differ from the folded form. Nodes are never reused: a parent's id is always below its children's,
//! and removal only marks, until `compact`.
use super::query::{fold_bytes, Query};
use std::path::{Component, Path, PathBuf};

pub const NONE: u32 = u32::MAX;
pub const DIR: u8 = 1;
pub const HIDDEN: u8 = 2;
const DEAD: u8 = 4;
const ROOT: u8 = 8;
const SAME: u8 = 16;
/// Past this the index reports itself incomplete instead of outgrowing its u32 offsets.
pub const MAX_NODES: usize = 30_000_000;

#[derive(Clone, Copy)]
struct Node {
    parent: u32,
    first_child: u32,
    next_sib: u32,
    fold_off: u32,
    raw_off: u32,
    mtime: u32,
    flags: u8,
}

#[derive(Default)]
pub struct Tree {
    nodes: Vec<Node>,
    fold: Vec<u8>,
    raw: Vec<u8>,
    pub roots: Vec<(PathBuf, u64)>,
    live: usize,
    dead: usize,
    /// The walk stopped at MAX_NODES: nothing can be promised about what's missing.
    pub truncated: bool,
}

#[derive(Clone, Copy)]
pub struct Hit {
    pub tier: u8,
    pub mtime: u32,
    pub id: u32,
}

#[cfg(unix)]
pub fn os_bytes(s: &std::ffi::OsStr) -> std::borrow::Cow<'_, [u8]> {
    use std::os::unix::ffi::OsStrExt;
    s.as_bytes().into()
}
#[cfg(not(unix))]
pub fn os_bytes(s: &std::ffi::OsStr) -> std::borrow::Cow<'_, [u8]> {
    // Names that aren't valid UTF-16 don't survive: `to_string_lossy` in `entry_from` can't open them
    // either, so the display check drops them, and the live walk shows them the same broken way.
    match s.to_string_lossy() {
        std::borrow::Cow::Borrowed(b) => b.as_bytes().into(),
        std::borrow::Cow::Owned(o) => o.into_bytes().into(),
    }
}
#[cfg(unix)]
pub fn bytes_os(b: &[u8]) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::OsStr::from_bytes(b).to_os_string()
}
#[cfg(not(unix))]
pub fn bytes_os(b: &[u8]) -> std::ffi::OsString {
    String::from_utf8_lossy(b).into_owned().into()
}

impl Tree {
    /// Roots come first, so their ids are 0..roots.len().
    pub fn new(roots: Vec<(PathBuf, u64)>) -> Tree {
        let mut t = Tree { roots: vec![], ..Default::default() };
        for (p, _) in &roots {
            t.push(NONE, &os_bytes(p.as_os_str()), 0, DIR | ROOT);
        }
        t.roots = roots;
        t
    }

    fn push(&mut self, parent: u32, raw: &[u8], mtime: u32, flags: u8) -> u32 {
        let id = self.nodes.len() as u32;
        let is_root = flags & ROOT != 0;
        let fold_off = self.fold.len() as u32;
        let mut flags = flags;
        let mut raw_off = self.raw.len() as u32;
        if is_root {
            self.raw.extend_from_slice(raw);
            self.raw.push(0);
        } else {
            let f = fold_bytes(raw);
            self.fold.extend_from_slice(&f);
            if f == raw {
                flags |= SAME;
                raw_off = fold_off;
            } else {
                self.raw.extend_from_slice(raw);
                self.raw.push(0);
            }
        }
        self.fold.push(0);
        let next_sib = if parent == NONE { NONE } else { self.nodes[parent as usize].first_child };
        self.nodes.push(Node { parent, first_child: NONE, next_sib, fold_off, raw_off, mtime, flags });
        if parent != NONE {
            self.nodes[parent as usize].first_child = id;
        }
        self.live += 1;
        id
    }

    /// A new child. The caller made sure `parent` has no live child of this name.
    pub fn add(&mut self, parent: u32, raw: &[u8], mtime: u32, flags: u8) -> u32 {
        self.push(parent, raw, mtime, flags & (DIR | HIDDEN))
    }

    /// A finished walk has no use for growing room.
    pub fn shrink(&mut self) {
        self.nodes.shrink_to_fit();
        self.fold.shrink_to_fit();
        self.raw.shrink_to_fit();
    }

    pub fn is_full(&self) -> bool {
        self.nodes.len() >= MAX_NODES
    }
    pub fn live_entries(&self) -> usize {
        self.live.saturating_sub(self.roots.len())
    }
    pub fn slots(&self) -> usize {
        self.nodes.len()
    }
    pub fn heap_bytes(&self) -> usize {
        self.nodes.capacity() * std::mem::size_of::<Node>() + self.fold.capacity() + self.raw.capacity()
    }
    pub fn is_dir(&self, id: u32) -> bool {
        self.nodes[id as usize].flags & DIR != 0
    }
    pub fn is_live(&self, id: usize) -> bool {
        self.nodes[id].flags & DEAD == 0
    }
    pub fn is_root(&self, id: usize) -> bool {
        self.nodes[id].flags & ROOT != 0
    }
    pub fn parent(&self, id: usize) -> u32 {
        self.nodes[id].parent
    }
    pub fn mtime(&self, id: usize) -> u32 {
        self.nodes[id].mtime
    }
    pub fn plain_flags(&self, id: usize) -> u8 {
        self.nodes[id].flags & (DIR | HIDDEN)
    }
    pub fn touch(&mut self, id: u32, mtime: u32, flags: u8) {
        let n = &mut self.nodes[id as usize];
        n.mtime = mtime;
        n.flags = (n.flags & !(DIR | HIDDEN)) | (flags & (DIR | HIDDEN));
    }

    fn fold_name(&self, id: usize) -> &[u8] {
        let start = self.nodes[id].fold_off as usize;
        let end = self.nodes.get(id + 1).map_or(self.fold.len(), |n| n.fold_off as usize);
        &self.fold[start..end - 1]
    }

    pub fn raw_name(&self, id: usize) -> &[u8] {
        let n = &self.nodes[id];
        if n.flags & SAME != 0 {
            return self.fold_name(id);
        }
        let tail = &self.raw[n.raw_off as usize..];
        &tail[..memchr::memchr(0, tail).unwrap_or(tail.len())]
    }

    /// Live children with their names.
    pub fn children(&self, id: u32) -> impl Iterator<Item = u32> + '_ {
        let mut cur = self.nodes[id as usize].first_child;
        std::iter::from_fn(move || {
            while cur != NONE {
                let c = cur;
                cur = self.nodes[c as usize].next_sib;
                if self.nodes[c as usize].flags & DEAD == 0 {
                    return Some(c);
                }
            }
            None
        })
    }

    pub fn child(&self, id: u32, raw: &[u8]) -> Option<u32> {
        self.children(id).find(|&c| self.raw_name(c as usize) == raw)
    }

    pub fn root_of(&self, path: &Path) -> Option<usize> {
        self.roots.iter().position(|(r, _)| path.starts_with(r))
    }

    /// The node at an absolute path, if the index holds one there.
    pub fn lookup(&self, path: &Path) -> Option<u32> {
        let r = self.root_of(path)?;
        let rest = path.strip_prefix(&self.roots[r].0).ok()?;
        let mut cur = r as u32;
        for c in rest.components() {
            let Component::Normal(name) = c else { return None };
            cur = self.child(cur, &os_bytes(name))?;
        }
        Some(cur)
    }

    pub fn remove(&mut self, id: u32) {
        if self.nodes[id as usize].flags & ROOT != 0 {
            let kids: Vec<u32> = self.children(id).collect();
            kids.into_iter().for_each(|k| self.remove(k));
            return;
        }
        let mut stack = vec![id];
        while let Some(i) = stack.pop() {
            if self.nodes[i as usize].flags & DEAD != 0 {
                continue;
            }
            stack.extend(self.children(i));
            self.nodes[i as usize].flags |= DEAD;
            self.live -= 1;
            self.dead += 1;
        }
    }

    /// Copy `other`, a tree with one root, in as a child of `parent` named `raw`.
    pub fn graft(&mut self, parent: u32, raw: &[u8], mtime: u32, flags: u8, other: &Tree) {
        let top = self.add(parent, raw, mtime, flags | DIR);
        let mut map = vec![NONE; other.nodes.len()];
        map[0] = top;
        for i in 1..other.nodes.len() {
            if other.is_live(i) {
                let p = map[other.nodes[i].parent as usize];
                if p != NONE {
                    map[i] = self.add(p, other.raw_name(i), other.nodes[i].mtime, other.nodes[i].flags);
                }
            }
        }
    }

    pub fn wants_compaction(&self) -> bool {
        self.dead > 10_000 && self.dead > self.live / 4
    }

    /// Drop what `remove` only marked.
    pub fn compact(&mut self) {
        let mut out = Tree::new(self.roots.clone());
        out.truncated = self.truncated;
        let mut map = vec![NONE; self.nodes.len()];
        for (i, m) in map.iter_mut().enumerate().take(self.roots.len()) {
            *m = i as u32;
        }
        for i in self.roots.len()..self.nodes.len() {
            if self.is_live(i) {
                map[i] = out.add(map[self.nodes[i].parent as usize], self.raw_name(i), self.nodes[i].mtime, self.nodes[i].flags);
            }
        }
        *self = out;
    }

    /// Path of `id` relative to the ancestor `scope`.
    pub fn rel_path(&self, id: u32, scope: u32) -> PathBuf {
        let mut parts = vec![];
        let mut cur = id;
        while cur != scope && cur != NONE {
            parts.push(cur);
            cur = self.nodes[cur as usize].parent;
        }
        parts.iter().rev().map(|&c| bytes_os(self.raw_name(c as usize))).collect()
    }

    /// The folder at `path` and every folder below it.
    pub fn dirs_under(&self, path: &Path) -> Vec<PathBuf> {
        let Some(top) = self.lookup(path).filter(|&id| self.is_dir(id)) else { return vec![] };
        let (mut out, mut stack) = (vec![], vec![(top, path.to_path_buf())]);
        while let Some((id, p)) = stack.pop() {
            for c in self.children(id).filter(|&c| self.is_dir(c)) {
                stack.push((c, p.join(bytes_os(self.raw_name(c as usize)))));
            }
            out.push(p);
        }
        out
    }

    pub fn dirs(&self) -> impl Iterator<Item = PathBuf> + '_ {
        (0..self.nodes.len()).filter(|&i| self.is_live(i) && self.nodes[i].flags & DIR != 0).map(|i| {
            let mut root = i as u32;
            while self.nodes[root as usize].parent != NONE {
                root = self.nodes[root as usize].parent;
            }
            self.roots[root as usize].0.join(self.rel_path(i as u32, root))
        })
    }

    /// Below `scope`, and (unless `hidden`) with nothing hidden between it and the scope: the live
    /// walk doesn't enter a hidden folder either, but the folder you search from doesn't count.
    fn visible_in(&self, id: u32, scope: u32, hidden: bool) -> bool {
        let mut cur = id;
        loop {
            if cur == scope {
                return true;
            }
            let n = &self.nodes[cur as usize];
            if !hidden && n.flags & HIDDEN != 0 || n.parent == NONE {
                return false;
            }
            cur = n.parent;
        }
    }

    /// All matches below `scope`. `None` if cancelled.
    pub fn search(&self, q: &Query, scope: u32, hidden: bool, cancelled: &dyn Fn() -> bool) -> Option<Vec<Hit>> {
        let mut hits = vec![];
        let mut consider = |id: usize, name: &[u8]| {
            if id as u32 != scope && self.is_live(id) && !self.is_root(id) && q.matches_folded(name) && self.visible_in(id as u32, scope, hidden) {
                hits.push(Hit { tier: q.tier(name), mtime: self.nodes[id].mtime, id: id as u32 });
            }
        };
        if let Some(longest) = q.words.iter().max_by_key(|w| w.len()) {
            let finder = memchr::memmem::Finder::new(longest);
            let mut pos = 0;
            let mut steps = 0u32;
            while let Some(found) = finder.find(&self.fold[pos..]) {
                let at = pos + found;
                let id = self.nodes.partition_point(|n| n.fold_off as usize <= at) - 1;
                let name = self.fold_name(id);
                consider(id, name);
                pos = self.nodes[id].fold_off as usize + name.len() + 1;
                steps += 1;
                if steps & 0x3FF == 0 && cancelled() {
                    return None;
                }
                if pos >= self.fold.len() {
                    break;
                }
            }
        } else {
            for id in 0..self.nodes.len() {
                if id & 0xFFFF == 0 && cancelled() {
                    return None;
                }
                consider(id, self.fold_name(id));
            }
        }
        Some(hits)
    }
}

/// Best first, at most `cap`: name match, then most recently modified.
pub fn rank(mut hits: Vec<Hit>, cap: usize) -> Vec<Hit> {
    let key = |h: &Hit| (h.tier, std::cmp::Reverse(h.mtime), h.id);
    if hits.len() > cap {
        hits.select_nth_unstable_by_key(cap, key);
        hits.truncate(cap);
    }
    hits.sort_unstable_by_key(key);
    hits
}
