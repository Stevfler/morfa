//! Quick Look for office files without an office suite: DOCX/ODT, PPTX/ODP and spreadsheets
//! (XLSX/XLS/XLSB/ODS via calamine) are read into a light block model the webview renders.
//! Only text structure survives (headings, lists, bold/italic runs, tables); layout, images
//! and styles are dropped. Sizes are capped so a huge or hostile file can't stall the UI.
use crate::CmdResult;
use quick_xml::events::{BytesStart, Event};
use serde::Serialize;
use std::{io::Read, path::Path};

/// Largest XML part read from a package (zip-bomb guard).
const MAX_PART: u64 = 32 << 20;
const MAX_BLOCKS: usize = 4000;
const MAX_ROWS: usize = 1000;
const MAX_COLS: usize = 60;
const MAX_SLIDES: usize = 300;


#[derive(Serialize, Default, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub b: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub i: bool,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Block {
    H { level: u8, runs: Vec<Run> },
    P { runs: Vec<Run> },
    Li { level: u8, runs: Vec<Run> },
    Table { rows: Vec<Vec<String>> },
}

#[derive(Serialize)]
pub struct Sheet {
    name: String,
    rows: Vec<Vec<String>>,
    /// Rows or columns were cut to fit the limits.
    truncated: bool,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Doc {
    Text { blocks: Vec<Block>, truncated: bool },
    Slides { slides: Vec<Vec<Block>> },
    Sheets { sheets: Vec<Sheet> },
}

#[tauri::command]
pub async fn office_preview(path: String) -> CmdResult<Doc> {
    tauri::async_runtime::spawn_blocking(move || read(Path::new(&path))).await.map_err(|e| e.to_string())?
}

pub fn read(path: &Path) -> CmdResult<Doc> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    match ext.as_str() {
        "docx" | "docm" => {
            let b = ooxml_text(&part(path, "word/document.xml")?);
            Ok(text_doc(b))
        }
        "odt" => Ok(text_doc(odf_text(&part(path, "content.xml")?).into_iter().flatten().collect())),
        "odp" => Ok(Doc::Slides { slides: odf_text(&part(path, "content.xml")?).into_iter().take(MAX_SLIDES).collect() }),
        "pptx" | "pptm" => pptx(path),
        "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" => sheets(path),
        _ => Err(format!("unsupported: .{ext}")),
    }
}

fn text_doc(mut blocks: Vec<Block>) -> Doc {
    let truncated = blocks.len() > MAX_BLOCKS;
    blocks.truncate(MAX_BLOCKS);
    Doc::Text { blocks, truncated }
}

fn open_zip(path: &Path) -> CmdResult<zip::ZipArchive<std::fs::File>> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    zip::ZipArchive::new(f).map_err(|e| format!("not an office package: {e}"))
}

fn read_entry(z: &mut zip::ZipArchive<std::fs::File>, name: &str) -> CmdResult<String> {
    let e = z.by_name(name).map_err(|_| format!("missing {name}"))?;
    let mut s = String::new();
    e.take(MAX_PART).read_to_string(&mut s).map_err(|e| e.to_string())?;
    Ok(s)
}

fn part(path: &Path, name: &str) -> CmdResult<String> {
    read_entry(&mut open_zip(path)?, name)
}

// ---------------------------------------------------------------------------
// Streaming XML walk shared by the parsers.

enum Ev<'a, 'x> {
    Open(&'a BytesStart<'x>),
    Close(&'a str),
    Text(&'a str),
}

fn walk(xml: &str, mut f: impl FnMut(Ev)) {
    let mut r = quick_xml::Reader::from_str(xml);
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => f(Ev::Open(&e)),
            Ok(Event::Empty(e)) => {
                f(Ev::Open(&e));
                f(Ev::Close(e.local_name().as_ref()));
            }
            Ok(Event::End(e)) => f(Ev::Close(e.local_name().as_ref())),
            Ok(Event::Text(t)) => f(Ev::Text(&t)),
            Ok(Event::CData(t)) => f(Ev::Text(&t)),
            Ok(Event::GeneralRef(r)) => {
                let s = match r.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    _ => quick_xml::escape::resolve_predefined_entity(&r).unwrap_or_default().to_string(),
                };
                f(Ev::Text(&s));
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
}

fn attr(e: &BytesStart, key: &str) -> Option<String> {
    let a = e.try_get_attribute(key).ok()??;
    Some(a.value.into_owned())
}

/// OOXML on/off attribute: `<w:b/>` is on unless `val` says otherwise.
fn on(e: &BytesStart, key: &str) -> bool {
    !matches!(attr(e, key).as_deref(), Some("0" | "false" | "off" | "none"))
}

/// Collects paragraphs and tables; paragraphs inside a table cell become cell text.
#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    runs: Vec<Run>,
    /// Open tables (nested ones flatten into their parent cell's text).
    tables: Vec<Vec<Vec<String>>>,
    cell: Option<String>,
    b: bool,
    i: bool,
}

impl Builder {
    fn text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        match self.runs.last_mut() {
            Some(r) if r.b == self.b && r.i == self.i => r.text.push_str(s),
            _ => self.runs.push(Run { text: s.into(), b: self.b, i: self.i }),
        }
    }
    fn end_para(&mut self, kind: Para) {
        let runs = std::mem::take(&mut self.runs);
        if let Some(cell) = &mut self.cell {
            let t: String = runs.iter().map(|r| r.text.as_str()).collect();
            if !cell.is_empty() && !t.is_empty() {
                cell.push('\n');
            }
            cell.push_str(&t);
            return;
        }
        if self.tables.is_empty() {
            self.blocks.push(match kind {
                Para::H(level) => Block::H { level, runs },
                Para::Li(level) => Block::Li { level, runs },
                Para::P => Block::P { runs },
            });
        }
    }
    fn open_table(&mut self) {
        self.tables.push(vec![]);
    }
    fn open_row(&mut self) {
        if let Some(t) = self.tables.last_mut() {
            t.push(vec![]);
        }
    }
    fn open_cell(&mut self) {
        if self.tables.len() == 1 {
            self.cell = Some(String::new());
        }
    }
    fn close_cell(&mut self) {
        if self.tables.len() != 1 {
            return;
        }
        let c = self.cell.take().unwrap_or_default();
        if let Some(row) = self.tables.last_mut().and_then(|t| t.last_mut()) {
            row.push(c);
        }
    }
    fn close_table(&mut self) {
        match self.tables.pop() {
            Some(rows) if self.tables.is_empty() => self.blocks.push(Block::Table { rows }),
            _ => {}
        }
    }
}

#[derive(Clone, Copy)]
enum Para {
    P,
    H(u8),
    Li(u8),
}

/// Heading level from an OOXML style id: Heading1..9, Title, or localized ids like "1".
fn heading_level(style: &str) -> Option<u8> {
    let s = style.to_ascii_lowercase();
    if s == "title" {
        return Some(1);
    }
    let digits = s.trim_start_matches(|c: char| !c.is_ascii_digit());
    let n: u8 = digits.parse().ok()?;
    let head = &s[..s.len() - digits.len()];
    ((head.is_empty() || head.contains("heading") || head.contains("заголовок")) && (1..=6).contains(&n)).then_some(n)
}

// ---------------------------------------------------------------------------
// DOCX (word/document.xml) and PPTX slides (DrawingML) — same run/paragraph shape.

fn ooxml_text(xml: &str) -> Vec<Block> {
    let mut b = Builder::default();
    let mut para = Para::P;
    let (mut in_rpr, mut in_t) = (false, false);
    walk(xml, |ev| match ev {
        Ev::Open(e) => match e.local_name().as_ref() {
            "p" => para = Para::P,
            "pStyle" => {
                if let Some(l) = attr(e, "w:val").as_deref().and_then(heading_level) {
                    para = Para::H(l);
                }
            }
            "ilvl" => para = Para::Li(attr(e, "w:val").and_then(|v| v.parse().ok()).unwrap_or(0)),
            "numPr" if matches!(para, Para::P) => para = Para::Li(0),
            "r" => (b.b, b.i) = (false, false),
            "rPr" => in_rpr = true,
            "b" if in_rpr => b.b = on(e, "w:val"),
            "i" if in_rpr => b.i = on(e, "w:val"),
            "t" => in_t = true,
            "tab" if !in_rpr => b.text("\t"),
            "br" | "cr" => b.text("\n"),
            "tbl" => b.open_table(),
            "tr" => b.open_row(),
            "tc" => b.open_cell(),
            _ => {}
        },
        Ev::Close(n) => match n {
            "p" => b.end_para(para),
            "rPr" => in_rpr = false,
            "t" => in_t = false,
            "tc" => b.close_cell(),
            "tbl" => b.close_table(),
            _ => {}
        },
        Ev::Text(s) if in_t => b.text(s),
        Ev::Text(_) => {}
    });
    b.blocks
}

fn pptx(path: &Path) -> CmdResult<Doc> {
    let mut z = open_zip(path)?;
    let mut names: Vec<(u32, String)> = z
        .file_names()
        .filter_map(|n| n.ok())
        .filter_map(|n| Some((n.strip_prefix("ppt/slides/slide")?.strip_suffix(".xml")?.parse().ok()?, n.to_string())))
        .collect();
    names.sort();
    let mut slides = vec![];
    for (_, n) in names.into_iter().take(MAX_SLIDES) {
        slides.push(slide(&read_entry(&mut z, &n)?));
    }
    Ok(Doc::Slides { slides })
}

/// One slide: paragraphs of title placeholders become headings; `a:rPr b="1"` marks bold.
fn slide(xml: &str) -> Vec<Block> {
    let mut b = Builder::default();
    let (mut title, mut in_t) = (false, false);
    let mut level = 0u8;
    walk(xml, |ev| match ev {
        Ev::Open(e) => match e.local_name().as_ref() {
            "sp" => title = false,
            "ph" => title = matches!(attr(e, "type").as_deref(), Some("title" | "ctrTitle")),
            "p" => level = 0,
            "pPr" => level = attr(e, "lvl").and_then(|v| v.parse().ok()).unwrap_or(0),
            "r" => (b.b, b.i) = (false, false),
            "rPr" => (b.b, b.i) = (attr(e, "b").as_deref() == Some("1"), attr(e, "i").as_deref() == Some("1")),
            "t" => in_t = true,
            "br" => b.text("\n"),
            "tbl" => b.open_table(),
            "tr" => b.open_row(),
            "tc" => b.open_cell(),
            _ => {}
        },
        Ev::Close(n) => match n {
            "p" => {
                if b.runs.iter().any(|r| !r.text.trim().is_empty()) || b.cell.is_some() {
                    b.end_para(if title { Para::H(1) } else if level > 0 { Para::Li(level - 1) } else { Para::P });
                } else {
                    b.runs.clear();
                }
            }
            "t" => in_t = false,
            "tc" => b.close_cell(),
            "tbl" => b.close_table(),
            _ => {}
        },
        Ev::Text(s) if in_t => b.text(s),
        Ev::Text(_) => {}
    });
    b.blocks
}

// ---------------------------------------------------------------------------
// ODF (content.xml). Returns blocks per `draw:page` for presentations, or one group for text.

fn odf_text(xml: &str) -> Vec<Vec<Block>> {
    let mut pages: Vec<Vec<Block>> = vec![];
    let mut b = Builder::default();
    let mut stack: Vec<Para> = vec![];
    let mut list_depth = 0u8;
    let mut in_body = false;
    // Text inside these elements isn't document content.
    let mut skip = 0u32;
    walk(xml, |ev| match ev {
        Ev::Open(e) => match e.local_name().as_ref() {
            "body" => in_body = true,
            "page" => {
                if !b.blocks.is_empty() || !pages.is_empty() {
                    pages.push(std::mem::take(&mut b.blocks));
                }
            }
            "notes" | "annotation" | "tracked-changes" => skip += 1,
            "list" => list_depth += 1,
            "h" => stack.push(Para::H(attr(e, "text:outline-level").and_then(|v| v.parse().ok()).unwrap_or(1).clamp(1, 6))),
            "p" => stack.push(if list_depth > 0 { Para::Li(list_depth - 1) } else { Para::P }),
            "s" if !stack.is_empty() => b.text(&" ".repeat(attr(e, "text:c").and_then(|v| v.parse().ok()).unwrap_or(1).min(64))),
            "tab" if !stack.is_empty() => b.text("\t"),
            "line-break" => b.text("\n"),
            "table" => b.open_table(),
            "table-row" => b.open_row(),
            "table-cell" => b.open_cell(),
            _ => {}
        },
        Ev::Close(n) => match n {
            "notes" | "annotation" | "tracked-changes" => skip = skip.saturating_sub(1),
            "list" => list_depth = list_depth.saturating_sub(1),
            "h" | "p" => {
                if let Some(p) = stack.pop() {
                    if skip == 0 {
                        b.end_para(p);
                    } else {
                        b.runs.clear();
                    }
                }
            }
            "table-cell" => b.close_cell(),
            "table" => b.close_table(),
            _ => {}
        },
        Ev::Text(s) if in_body && skip == 0 && !stack.is_empty() => b.text(s),
        Ev::Text(_) => {}
    });
    pages.push(b.blocks);
    pages
}

// ---------------------------------------------------------------------------
// Spreadsheets.

fn sheets(path: &Path) -> CmdResult<Doc> {
    use calamine::{open_workbook_auto, Data, Reader};
    let mut wb = open_workbook_auto(path).map_err(|e| e.to_string())?;
    let mut out = vec![];
    for name in wb.sheet_names() {
        let Ok(range) = wb.worksheet_range(&name) else { continue };
        let (h, w) = range.get_size();
        let mut rows: Vec<Vec<String>> = range
            .rows()
            .take(MAX_ROWS)
            .map(|r| {
                r.iter()
                    .take(MAX_COLS)
                    .map(|c| match c {
                        Data::Empty => String::new(),
                        Data::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", *f as i64),
                        Data::DateTime(d) if d.is_datetime() => {
                            let (y, mo, da, h, mi, ..) = d.to_ymd_hms_milli();
                            if (h, mi) == (0, 0) { format!("{y}-{mo:02}-{da:02}") } else { format!("{y}-{mo:02}-{da:02} {h:02}:{mi:02}") }
                        }
                        _ => c.to_string(),
                    })
                    .collect()
            })
            .collect();
        // Trailing empty columns/rows come from formatting; drop them.
        while rows.last().is_some_and(|r| r.iter().all(String::is_empty)) {
            rows.pop();
        }
        let used = rows.iter().map(|r| r.iter().rposition(|c| !c.is_empty()).map_or(0, |i| i + 1)).max().unwrap_or(0);
        rows.iter_mut().for_each(|r| r.truncate(used));
        out.push(Sheet { name, rows, truncated: h > MAX_ROWS || w > MAX_COLS });
    }
    Ok(Doc::Sheets { sheets: out })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(b: &Block) -> String {
        match b {
            Block::H { runs, .. } | Block::P { runs } | Block::Li { runs, .. } => runs.iter().map(|r| r.text.as_str()).collect(),
            Block::Table { rows } => format!("{rows:?}"),
        }
    }

    #[test]
    fn docx_structure() {
        let xml = r#"<w:document xmlns:w="w"><w:body>
          <w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Title &amp; more</w:t></w:r></w:p>
          <w:p><w:r><w:rPr><w:b/></w:rPr><w:t>bold</w:t></w:r><w:r><w:rPr><w:b w:val="0"/></w:rPr><w:t xml:space="preserve"> plain</w:t></w:r></w:p>
          <w:p><w:pPr><w:numPr><w:ilvl w:val="1"/></w:numPr></w:pPr><w:r><w:t>item</w:t></w:r></w:p>
          <w:tbl><w:tr><w:tc><w:p><w:r><w:t>a</w:t></w:r></w:p><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>c</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
        </w:body></w:document>"#;
        let b = ooxml_text(xml);
        assert_eq!(b.len(), 4);
        assert!(matches!(b[0], Block::H { level: 2, .. }));
        assert_eq!(runs(&b[0]), "Title & more");
        let Block::P { runs: r } = &b[1] else { panic!() };
        assert_eq!((r[0].b, r[1].b, r[1].text.as_str()), (true, false, " plain"));
        assert!(matches!(b[2], Block::Li { level: 1, .. }));
        assert_eq!(b[3], Block::Table { rows: vec![vec!["a\nb".into(), "c".into()]] });
    }

    #[test]
    fn odf_pages_and_lists() {
        let xml = r#"<office:document-content><office:body><office:presentation>
          <draw:page><draw:frame><text:p>One<text:s text:c="2"/>x</text:p></draw:frame></draw:page>
          <draw:page><text:list><text:list-item><text:p>Item</text:p></text:list-item></text:list><presentation:notes><text:p>hidden</text:p></presentation:notes></draw:page>
        </office:presentation></office:body></office:document-content>"#;
        let p = odf_text(xml);
        assert_eq!(p.len(), 2);
        assert_eq!(runs(&p[0][0]), "One  x");
        assert_eq!(p[1], vec![Block::Li { level: 0, runs: vec![Run { text: "Item".into(), ..Default::default() }] }]);
    }

    #[test]
    fn pptx_slide() {
        let xml = r#"<p:sld><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>Hello</a:t></a:r></a:p></p:txBody></p:sp>
          <p:sp><p:nvSpPr><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:rPr b="1"/><a:t>Point</a:t></a:r></a:p><a:p><a:pPr lvl="1"/><a:r><a:t>Sub</a:t></a:r></a:p><a:p/></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let b = slide(xml);
        assert_eq!(b.len(), 3);
        assert!(matches!(b[0], Block::H { level: 1, .. }));
        assert!(matches!(&b[1], Block::P { runs } if runs[0].b));
        assert!(matches!(b[2], Block::Li { level: 0, .. }));
    }

    #[test]
    fn heading_ids() {
        assert_eq!(heading_level("Heading3"), Some(3));
        assert_eq!(heading_level("1"), Some(1));
        assert_eq!(heading_level("Title"), Some(1));
        assert_eq!(heading_level("Normal"), None);
        assert_eq!(heading_level("TOC1"), None);
    }

    /// `MORFA_DOC=/path/file.docx cargo test docview::tests::real_file -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_file() {
        let Ok(p) = std::env::var("MORFA_DOC") else { return };
        let t = std::time::Instant::now();
        let d = read(Path::new(&p)).unwrap();
        let json = serde_json::to_string(&d).unwrap();
        let summary = match &d {
            Doc::Text { blocks, .. } => format!("text: {} blocks", blocks.len()),
            Doc::Slides { slides } => format!("slides: {}", slides.len()),
            Doc::Sheets { sheets } => format!("sheets: {:?}", sheets.iter().map(|s| (s.rows.len(), s.rows.first().map_or(0, Vec::len))).collect::<Vec<_>>()),
        };
        println!("{summary}, {} KB json, {:?}", json.len() / 1024, t.elapsed());
    }
}
