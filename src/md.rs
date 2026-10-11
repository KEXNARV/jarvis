//! Markdown de las respuestas, dibujado para la terminal: títulos, párrafos, listas anidadas
//! y numeradas, tareas, citas, código con su lenguaje, tablas con columnas que caben en el
//! ancho, enlaces, cursiva, tachado y líneas horizontales.
//!
//! Cada fila sale con `(línea, skip, cont)`, lo que necesita la selección del mouse: `skip`
//! son las columnas de decoración que no se copian y `cont` marca las filas que continúan la
//! anterior (al copiar se unen con un espacio en vez de un salto).

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme;
use crate::ui::GREEN;

pub type Row = (Line<'static>, usize, bool);

#[derive(Clone)]
struct Seg {
    text: String,
    style: Style,
}

/// Lo que va delante de cada fila según dónde está: sangría de listas y barras de cita.
#[derive(Clone)]
enum Frame {
    Quote,
    /// Ítem de lista: el ancho de su marca, para que las líneas siguientes queden colgadas.
    Item(usize),
}

struct List {
    next: Option<u64>,
}

struct Table {
    aligns: Vec<Alignment>,
    head: Vec<Vec<Seg>>,
    rows: Vec<Vec<Vec<Seg>>>,
    row: Vec<Vec<Seg>>,
    cell: Vec<Seg>,
    in_head: bool,
}

struct R {
    width: usize,
    out: Vec<Row>,
    segs: Vec<Seg>,
    frames: Vec<Frame>,
    lists: Vec<List>,
    /// La marca del ítem recién abierto («• », «3. », «○ »): va en su primera fila.
    marker: Option<String>,
    bold: u32,
    italic: u32,
    strike: u32,
    link: Option<String>,
    heading: Option<HeadingLevel>,
    code: Option<(String, String)>,
    table: Option<Table>,
}

pub fn render(text: &str, width: usize) -> Vec<Row> {
    let mut r = R {
        width: width.max(20),
        out: vec![],
        segs: vec![],
        frames: vec![],
        lists: vec![],
        marker: None,
        bold: 0,
        italic: 0,
        strike: 0,
        link: None,
        heading: None,
        code: None,
        table: None,
    };
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for ev in Parser::new_ext(text, opts) {
        r.event(ev);
    }
    r.flush();
    if let Some((lang, body)) = r.code.take() {
        r.code_block(&lang, &body); // bloque todavía abierto mientras llega el streaming
    }
    while r.out.last().is_some_and(|l| is_blank(&l.0)) {
        r.out.pop();
    }
    r.out
}

fn is_blank(l: &Line) -> bool {
    l.spans.iter().all(|s| s.content.trim().is_empty())
}

impl R {
    fn style(&self) -> Style {
        let mut s = Style::new().fg(theme::text());
        if self.heading.is_some() {
            s = s.fg(match self.heading {
                Some(HeadingLevel::H1 | HeadingLevel::H2) => theme::accent(),
                _ => Color::White,
            });
            s = s.add_modifier(Modifier::BOLD);
        }
        if self.bold > 0 {
            s = s.fg(Color::White).add_modifier(Modifier::BOLD);
        }
        if self.italic > 0 {
            s = s.add_modifier(Modifier::ITALIC);
        }
        if self.strike > 0 {
            s = s.fg(theme::faint()).add_modifier(Modifier::CROSSED_OUT);
        }
        if self.link.is_some() {
            s = s.fg(theme::accent()).add_modifier(Modifier::UNDERLINED);
        }
        if self.frames.iter().any(|f| matches!(f, Frame::Quote)) && self.bold == 0 {
            s = s.fg(Color::Rgb(160, 180, 190)).add_modifier(Modifier::ITALIC);
        }
        s
    }

    /// Texto corrido: las URL sueltas salen como enlace (subrayadas, en el acento), porque un
    /// clic sobre ellas las abre (enlace.rs).
    fn texto(&mut self, t: &str) {
        let base = self.style();
        if self.link.is_some() {
            return self.push(t, base);
        }
        let mut rest = t;
        for url in crate::enlace::urls(t) {
            let Some(i) = rest.find(&url) else { continue };
            if i > 0 {
                self.push(&rest[..i], base);
            }
            self.push(&url, base.fg(theme::accent()).add_modifier(Modifier::UNDERLINED));
            rest = &rest[i + url.len()..];
        }
        if !rest.is_empty() {
            self.push(rest, base);
        }
    }

    fn push(&mut self, text: &str, style: Style) {
        let seg = Seg { text: text.to_string(), style };
        match &mut self.table {
            Some(t) => t.cell.push(seg),
            None => self.segs.push(seg),
        }
    }

    /// Prefijo de las filas: sangría (no se copia) y barras de cita.
    fn prefix(&self, first: bool) -> (Vec<Span<'static>>, usize) {
        let mut spans = vec![Span::raw("  ")];
        let mut w = 2;
        let last_item = self.frames.iter().rposition(|f| matches!(f, Frame::Item(_)));
        for (i, f) in self.frames.iter().enumerate() {
            match f {
                Frame::Quote => {
                    spans.push(Span::styled("▎ ", Style::new().fg(theme::dim())));
                    w += 2;
                }
                // La marca del ítem más interno va en su primera fila; en las demás, el hueco.
                Frame::Item(mw) if Some(i) == last_item && first && self.marker.is_some() => {
                    let _ = mw;
                }
                Frame::Item(mw) => {
                    spans.push(Span::raw(" ".repeat(*mw)));
                    w += mw;
                }
            }
        }
        (spans, w)
    }

    fn blank(&mut self) {
        if self.out.last().is_some_and(|l| !is_blank(&l.0)) {
            self.out.push((Line::from(""), 0, false));
        }
    }

    /// Separación antes de un bloque: solo fuera de las listas, que van juntas.
    fn gap(&mut self) {
        if self.lists.is_empty() {
            self.blank();
        }
    }

    fn flush(&mut self) {
        if self.segs.is_empty() && self.marker.is_none() {
            return;
        }
        let segs = std::mem::take(&mut self.segs);
        let marker = self.marker.clone();
        let (p0, w0) = self.prefix(true);
        let (pn, wn) = self.prefix(false);
        let mw = marker.as_ref().map_or(0, |m| m.width());
        let room = self.width.saturating_sub(wn.max(w0 + mw)).max(10);
        let lines = wrap(&segs, room);
        for (i, line) in lines.into_iter().enumerate() {
            let mut spans = if i == 0 { p0.clone() } else { pn.clone() };
            let skip = if i == 0 { w0 } else { wn };
            if i == 0 {
                if let Some(m) = &marker {
                    spans.push(Span::styled(m.clone(), Style::new().fg(theme::accent())));
                }
            }
            spans.extend(line.into_iter().map(|s| Span::styled(s.text, s.style)));
            self.out.push((Line::from(spans), skip, i > 0));
        }
        self.marker = None;
    }

    fn event(&mut self, ev: Event) {
        if let Some((_, body)) = &mut self.code {
            match ev {
                Event::Text(t) => body.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, body) = self.code.take().unwrap();
                    self.code_block(&lang, &body);
                }
                _ => {}
            }
            return;
        }
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.texto(&t),
            Event::Code(t) => {
                let st = Style::new().fg(GREEN);
                self.push(&t, st);
            }
            Event::SoftBreak => self.push(" ", self.style()),
            Event::HardBreak => self.flush(),
            Event::Rule => {
                self.flush();
                self.gap();
                let (mut spans, w) = self.prefix(false);
                let n = self.width.saturating_sub(w).min(60);
                spans.push(Span::styled("─".repeat(n), Style::new().fg(theme::dim())));
                self.out.push((Line::from(spans), usize::MAX, false));
                self.gap();
            }
            Event::TaskListMarker(done) => {
                let m = if done { "✓ " } else { "○ " };
                self.marker = Some(m.into());
                if let Some(Frame::Item(w)) = self.frames.last_mut() {
                    *w = 2;
                }
            }
            Event::Html(t) | Event::InlineHtml(t) => self.push(&t, Style::new().fg(theme::faint())),
            Event::FootnoteReference(t) => self.push(&format!("[{t}]"), Style::new().fg(theme::faint())),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                self.flush();
                // Dentro de un ítem, el primer párrafo va en la fila de la marca.
                if self.marker.is_none() {
                    self.gap();
                }
            }
            Tag::Heading { level, .. } => {
                self.flush();
                self.blank();
                self.heading = Some(level);
                if level >= HeadingLevel::H3 {
                    self.push("", self.style());
                }
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.gap();
                self.frames.push(Frame::Quote);
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                self.gap();
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.split([',', ' ']).next().unwrap_or("").to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::List(start) => {
                self.flush();
                if self.lists.is_empty() {
                    self.blank();
                }
                self.lists.push(List { next: start });
            }
            Tag::Item => {
                self.flush();
                // El símbolo cambia por nivel de viñetas; las numeradas no cuentan.
                let depth = self.lists.iter().filter(|l| l.next.is_none()).count().saturating_sub(1);
                let marker = match self.lists.last_mut().and_then(|l| l.next.as_mut()) {
                    Some(n) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    None => format!("{} ", ["•", "◦", "▪"][depth % 3]),
                };
                self.frames.push(Frame::Item(marker.width()));
                self.marker = Some(marker);
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => self.link = Some(dest_url.to_string()),
            Tag::Image { dest_url, .. } => self.push(&format!("[imagen: {dest_url}]"), Style::new().fg(theme::faint())),
            Tag::Table(aligns) => {
                self.flush();
                self.gap();
                self.table = Some(Table { aligns, head: vec![], rows: vec![], row: vec![], cell: vec![], in_head: false });
            }
            Tag::TableHead => {
                if let Some(t) = &mut self.table {
                    t.in_head = true;
                }
            }
            Tag::TableRow | Tag::TableCell => {}
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush(),
            TagEnd::Heading(level) => {
                let w: usize = self.segs.iter().map(|s| s.text.width()).sum();
                self.flush();
                self.heading = None;
                if level == HeadingLevel::H1 {
                    let n = w.min(self.width.saturating_sub(2));
                    self.out.push((
                        Line::from(vec![Span::raw("  "), Span::styled("━".repeat(n), Style::new().fg(theme::dim()))]),
                        usize::MAX,
                        false,
                    ));
                }
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.frames.pop();
                self.gap();
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Item => {
                self.flush();
                self.frames.pop();
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                if let Some(url) = self.link.take() {
                    let shown: String = match &self.table {
                        Some(t) => t.cell.iter().map(|s| s.text.as_str()).collect(),
                        None => self.segs.iter().map(|s| s.text.as_str()).collect(),
                    };
                    let bare = url.trim_start_matches("mailto:");
                    if !shown.ends_with(bare) {
                        self.push(&format!(" ({url})"), Style::new().fg(theme::dim()));
                    }
                }
            }
            TagEnd::TableCell => {
                if let Some(t) = &mut self.table {
                    let cell = std::mem::take(&mut t.cell);
                    t.row.push(cell);
                }
            }
            TagEnd::TableHead => {
                if let Some(t) = &mut self.table {
                    t.head = std::mem::take(&mut t.row);
                    t.in_head = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = &mut self.table {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                }
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    self.table_block(t);
                }
                self.gap();
            }
            _ => {}
        }
    }

    fn code_block(&mut self, lang: &str, body: &str) {
        let (base, w) = self.prefix(false);
        let dim = Style::new().fg(theme::dim());
        let mut head = base.clone();
        head.push(Span::styled("╭─", dim));
        if !lang.is_empty() {
            head.push(Span::styled(format!(" {lang} "), Style::new().fg(theme::faint())));
        }
        self.out.push((Line::from(head), usize::MAX, false));
        let room = self.width.saturating_sub(w + 2).max(10);
        for line in body.trim_end_matches('\n').lines() {
            let line = line.replace('\t', "    ");
            // El código no se reparte por palabras: se corta justo al ancho y se marca la
            // continuación con «↪», para no confundirla con una línea nueva.
            for (i, piece) in hard_wrap(&line, room.saturating_sub(2)).into_iter().enumerate() {
                let mut spans = base.clone();
                spans.push(Span::styled(if i == 0 { "│ " } else { "│↪" }, dim));
                spans.push(Span::styled(piece, Style::new().fg(GREEN)));
                self.out.push((Line::from(spans), w + 2, i > 0));
            }
        }
        let mut foot = base;
        foot.push(Span::styled("╰─", dim));
        self.out.push((Line::from(foot), usize::MAX, false));
    }

    fn table_block(&mut self, t: Table) {
        let n = t.head.len().max(t.rows.iter().map(Vec::len).max().unwrap_or(0));
        if n == 0 {
            return;
        }
        let cell_w = |c: &Vec<Seg>| c.iter().map(|s| s.text.width()).sum::<usize>();
        let mut widths: Vec<usize> = (0..n)
            .map(|j| {
                std::iter::once(&t.head).chain(&t.rows).filter_map(|r| r.get(j)).map(cell_w).max().unwrap_or(0).max(1)
            })
            .collect();
        let (base, w) = self.prefix(false);
        let avail = self.width.saturating_sub(w + 3 * (n - 1)).max(n * 4);
        // Si no cabe, se angosta la columna más ancha una y otra vez; el texto se reparte en
        // varias filas dentro de su celda.
        while widths.iter().sum::<usize>() > avail {
            let (j, &m) = widths.iter().enumerate().max_by_key(|(_, w)| **w).unwrap();
            if m <= 4 {
                break;
            }
            widths[j] = m - 1;
        }
        let sep = Style::new().fg(theme::dim());
        let bold = Style::new().fg(Color::White).add_modifier(Modifier::BOLD);
        let rows: Vec<(bool, &Vec<Vec<Seg>>)> =
            std::iter::once((true, &t.head)).chain(t.rows.iter().map(|r| (false, r))).collect();
        let wrapped: Vec<(bool, Vec<Vec<Vec<Seg>>>)> = rows
            .iter()
            .map(|(h, r)| {
                let cells = (0..n)
                    .map(|j| {
                        let c = r.get(j).cloned().unwrap_or_default();
                        let c = if *h { c.into_iter().map(|s| Seg { style: bold, ..s }).collect() } else { c };
                        wrap(&c, widths[j])
                    })
                    .collect();
                (*h, cells)
            })
            .collect();
        let tall = wrapped.iter().skip(1).any(|(_, cells)| cells.iter().any(|c| c.len() > 1));
        let rule = |cross: &str| {
            let mut spans = base.clone();
            let parts: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
            spans.push(Span::styled(parts.join(&format!("─{cross}─")), sep));
            (Line::from(spans), usize::MAX, false)
        };
        for (k, (head, cells)) in wrapped.iter().enumerate() {
            if k > 1 && tall {
                self.out.push(rule("┼"));
            }
            let h = cells.iter().map(Vec::len).max().unwrap_or(1).max(1);
            for line in 0..h {
                let mut spans = base.clone();
                for (j, c) in cells.iter().enumerate() {
                    if j > 0 {
                        spans.push(Span::styled(" │ ", sep));
                    }
                    let segs = c.get(line).cloned().unwrap_or_default();
                    let used: usize = segs.iter().map(|s| s.text.width()).sum();
                    let pad = widths[j].saturating_sub(used);
                    let (l, r) = match t.aligns.get(j) {
                        Some(Alignment::Right) => (pad, 0),
                        Some(Alignment::Center) => (pad / 2, pad - pad / 2),
                        _ => (0, pad),
                    };
                    spans.push(Span::raw(" ".repeat(l)));
                    spans.extend(segs.into_iter().map(|s| Span::styled(s.text, s.style)));
                    spans.push(Span::raw(" ".repeat(r)));
                }
                self.out.push((Line::from(spans), w, line > 0));
            }
            if *head {
                self.out.push(rule("┼"));
            }
        }
    }
}

/// Reparte segmentos con estilo en filas de `width` columnas, cortando entre palabras; una
/// palabra más larga que la fila se corta donde toca.
fn wrap(segs: &[Seg], width: usize) -> Vec<Vec<Seg>> {
    let width = width.max(1);
    let mut lines: Vec<Vec<Seg>> = vec![vec![]];
    let mut col = 0;
    let put = |lines: &mut Vec<Vec<Seg>>, text: &str, style: Style| {
        let line = lines.last_mut().unwrap();
        match line.last_mut() {
            Some(s) if s.style == style => s.text.push_str(text),
            _ => line.push(Seg { text: text.to_string(), style }),
        }
    };
    for seg in segs {
        for word in split_words(&seg.text) {
            let ww = word.width();
            if word.trim().is_empty() {
                if col > 0 && col + ww <= width {
                    put(&mut lines, word, seg.style);
                    col += ww;
                }
                continue;
            }
            if col + ww > width && col > 0 {
                // El espacio que quedó al final de la fila no se ve ni se copia.
                if let Some(s) = lines.last_mut().unwrap().last_mut() {
                    let t = s.text.trim_end().to_string();
                    s.text = t;
                }
                lines.push(vec![]);
                col = 0;
            }
            if ww <= width {
                put(&mut lines, word, seg.style);
                col += ww;
            } else {
                for piece in hard_wrap(word, width) {
                    if col > 0 {
                        lines.push(vec![]);
                    }
                    col = piece.width();
                    put(&mut lines, &piece, seg.style);
                }
            }
        }
    }
    lines
}

fn split_words(s: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut start = 0;
    let mut space = None;
    for (i, c) in s.char_indices() {
        let is = c == ' ';
        if space.is_some_and(|sp| sp != is) {
            out.push(&s[start..i]);
            start = i;
        }
        space = Some(is);
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

fn hard_wrap(s: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = vec![String::new()];
    let mut col = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if col + cw > width && col > 0 {
            out.push(String::new());
            col = 0;
        }
        out.last_mut().unwrap().push(c);
        col += cw;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|(l, _, _)| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>().trim_end().to_string()).collect()
    }

    #[test]
    fn listas_anidadas_y_numeradas() {
        let t = text(&render("1. uno\n2. dos\n   - a\n   - b\n     - c\n3. tres", 60));
        assert_eq!(t, vec!["  1. uno", "  2. dos", "     • a", "     • b", "       ◦ c", "  3. tres"]);
    }

    #[test]
    fn la_lista_cuelga_las_lineas_partidas() {
        let t = text(&render("- una frase bastante larga que no cabe en el ancho", 24));
        assert_eq!(t[0], "  • una frase bastante");
        assert!(t[1].starts_with("    "), "{t:?}");
    }

    #[test]
    fn tabla_con_columnas_alineadas() {
        let t = text(&render("| a | bb |\n|---|--:|\n| ccc | d |", 60));
        assert_eq!(t, vec!["  a   │ bb", "  ────┼───", "  ccc │  d"]);
    }

    #[test]
    fn tabla_ancha_se_reparte_en_filas() {
        let md = "| col | descripción |\n|---|---|\n| x | una descripción muy larga que no entra en la pantalla de ninguna manera |";
        let rows = render(md, 40);
        for (l, _, _) in &rows {
            assert!(l.width() <= 40, "{:?}", text(&rows));
        }
        assert!(rows.len() > 3);
    }

    #[test]
    fn codigo_con_lenguaje_y_corte() {
        let t = text(&render("```rust\nfn main() {}\n```", 60));
        assert_eq!(t, vec!["  ╭─ rust", "  │ fn main() {}", "  ╰─"]);
        let largo = format!("```\n{}\n```", "x".repeat(80));
        let t = text(&render(&largo, 40));
        assert!(t[2].starts_with("  │↪"), "{t:?}");
    }

    #[test]
    fn enlaces_tareas_y_citas() {
        let t = text(&render("[docs](https://x.y) y <https://a.b>", 80));
        assert_eq!(t[0], "  docs (https://x.y) y https://a.b");
        let t = text(&render("- [x] hecho\n- [ ] falta", 80));
        assert_eq!(t, vec!["  ✓ hecho", "  ○ falta"]);
        let t = text(&render("> ojo con esto", 80));
        assert_eq!(t, vec!["  ▎ ojo con esto"]);
    }

    #[test]
    fn parrafos_separados_y_bloque_abierto_en_streaming() {
        let t = text(&render("uno\n\ndos", 80));
        assert_eq!(t, vec!["  uno", "", "  dos"]);
        let t = text(&render("```sh\nls", 80));
        assert_eq!(t, vec!["  ╭─ sh", "  │ ls", "  ╰─"]);
    }
}

/// `IRIS_SNAPSHOT=1 cargo test md::snapshot` deja target/md.html con una respuesta de muestra.
#[cfg(test)]
#[test]
fn snapshot() {
    if !crate::rutas::hay_var("SNAPSHOT") {
        return;
    }
    crate::theme::poll();
    let sample = "Ya está: el NÚCLEO vivo quedó **integrado** en Iris. Lo verifiqué con `cargo test`.\n\n\
## Qué cambió\n\n\
1. **Colores por familia:**\n   - Celestes: presencia.\n   - Ámbar: tu turno.\n     - incluye *no te oigo*\n2. **Transiciones** con gesto.\n3. Contra el parpadeo: un Read de 50 ms se ve 0,6 s.\n\n\
| Estado | Qué hace | Cuándo, en Iris |\n|---|---|---|\n| En reposo | Respira lento, sin arcos, le salen «z» | más de 2 min sin actividad |\n| Te leo | Mira hacia la entrada y da un respingo con cada tecla | mientras escribes |\n| Git | Crece un grafo de commits | Bash con `git` |\n\n\
```rust\nlet tick = Duration::from_millis(16); // 60 fps: lo que más se mueve es el núcleo, y conviene que no se trabe\n```\n\n\
> Ojo: los cambios no están commiteados.\n\n---\n\n\
- [x] markdown nuevo\n- [ ] commit\n\nMás en [el repo](https://github.com/KEXNARV/iris) y ~~nada más~~.";
    let rows = render(sample, 78);
    let mut html = String::from("<!doctype html><meta charset=utf-8><body style='background:#0b1218;padding:16px'><pre style='font-family:\"JetBrainsMono Nerd Font\",monospace;font-size:15px;line-height:1.35;color:#cde1eb'>");
    for (l, _, _) in rows {
        for sp in l.spans {
            let c = match sp.style.fg {
                Some(Color::Rgb(r, g, b)) => format!("rgb({r},{g},{b})"),
                Some(Color::White) => "#fff".into(),
                _ => "inherit".into(),
            };
            let m = sp.style.add_modifier;
            let mut st = format!("color:{c};");
            if m.contains(Modifier::BOLD) { st.push_str("font-weight:bold;"); }
            if m.contains(Modifier::ITALIC) { st.push_str("font-style:italic;"); }
            if m.contains(Modifier::UNDERLINED) { st.push_str("text-decoration:underline;"); }
            if m.contains(Modifier::CROSSED_OUT) { st.push_str("text-decoration:line-through;"); }
            let t = sp.content.replace('&', "&amp;").replace('<', "&lt;");
            html.push_str(&format!("<span style='{st}'>{t}</span>"));
        }
        html.push('\n');
    }
    std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/target/md.html"), html).unwrap();
}
