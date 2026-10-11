//! Enlaces del chat: un clic sobre una URL la abre en el navegador. Con el mouse en manos de
//! Iris la terminal ya no abre nada sola, así que lo hacemos nosotros.
//!
//! No hace falta saber dónde quedó cada enlace en pantalla: el pedazo de texto bajo el clic
//! (más los de las filas vecinas, porque una URL más larga que la línea se parte en varias) se
//! busca entre las URL de la conversación, que están enteras en el texto de los mensajes.

use std::process::{Command, Stdio};

use crate::select::View;

/// Las URL http(s) de un texto, enteras y sin repetir. También las de los enlaces markdown
/// (`[texto](url)`) y las de `<url>`.
pub fn urls(text: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut rest = text;
    while let Some(i) = rest.find("http") {
        let tail = &rest[i..];
        if !(tail.starts_with("https://") || tail.starts_with("http://")) {
            rest = &tail[4..];
            continue;
        }
        let end = tail.find(|c: char| c.is_whitespace() || "<>\"'`)]".contains(c)).unwrap_or(tail.len());
        let url = tail[..end].trim_end_matches(|c: char| ".,;:!?*_".contains(c));
        if url.len() > "https://".len() && !out.iter().any(|u| u == url) {
            out.push(url.to_string());
        }
        rest = &tail[end.max(1)..];
    }
    out
}

/// La URL bajo (fila, columna) de la vista, si la hay, buscada entre `urls`.
pub fn bajo(view: &View, (r, c): (usize, usize), urls: &[String]) -> Option<String> {
    let row = view.rows.get(r)?;
    let (tok, empieza, termina) = palabra(&row.text, c.max(row.skip))?;
    // Los pedazos que pueden formar la URL: el de la fila y, si la URL se partió, los de las
    // filas de antes y de después (sin espacio entre medio, como los dejó el corte). Se sigue
    // mientras el pedazo ocupe la fila entera: así cruza una URL partida en tres o más.
    let mut trozo = tok.clone();
    let (mut i, mut toca) = (r, empieza);
    while toca && view.rows[i].cont && i > 0 {
        let p = &view.rows[i - 1];
        let Some(prev) = ultima(&p.text, p.skip) else { break };
        toca = sola(&p.text, p.skip);
        trozo = prev + &trozo;
        i -= 1;
    }
    let (mut j, mut toca) = (r, termina);
    while toca {
        let Some(n) = view.rows.get(j + 1).filter(|n| n.cont) else { break };
        let Some(next) = primera(&n.text, n.skip) else { break };
        toca = sola(&n.text, n.skip);
        trozo.push_str(&next);
        j += 1;
    }
    const BORDE: &str = "([<«\"'`)]>».,;:!?*_";
    // Lo que va pegado delante de la URL no es parte de ella: el `texto](` de un enlace
    // markdown en crudo (el subtítulo del Cine lo muestra así), un `(` o un `<`.
    let desde_url = |s: &str| s.find("http").map_or(s.to_string(), |i| s[i..].to_string());
    let limpio = desde_url(&trozo);
    let limpio = limpio.trim_matches(|c: char| BORDE.contains(c));
    let tok = desde_url(&tok);
    let tok = tok.trim_matches(|c: char| BORDE.contains(c));
    // Una palabra suelta que además aparece dentro de una URL («tienda») no es el enlace: lo
    // que se tocó tiene que ser parte de una URL escrita, con su esquema.
    if tok.is_empty() || !limpio.contains("://") {
        return None;
    }
    // El pedazo tocado tiene que estar en la URL; entre las que lo tienen, la que encaja con
    // el pedazo reconstruido (lo de antes de la URL o lo de después, como `)`, ya se limpió).
    urls.iter()
        .filter(|u| u.contains(tok))
        .find(|u| u.contains(limpio) || limpio.contains(u.as_str()))
        .cloned()
}

/// Si la fila tiene una sola palabra (sin espacios después de la decoración): un pedazo de
/// una palabra más larga que la línea, cortada sin espacio.
fn sola(text: &str, skip: usize) -> bool {
    let t: String = text.chars().skip(skip).collect();
    !t.trim().is_empty() && !t.trim().contains(char::is_whitespace)
}

/// La palabra (sin espacios) que ocupa la columna `col`, y si toca el principio y el final del
/// texto de la fila.
fn palabra(text: &str, col: usize) -> Option<(String, bool, bool)> {
    use unicode_width::UnicodeWidthChar;
    let chars: Vec<char> = text.chars().collect();
    let mut x = 0;
    let mut at = None;
    for (i, ch) in chars.iter().enumerate() {
        let w = ch.width().unwrap_or(0);
        if col < x + w.max(1) {
            at = Some(i);
            break;
        }
        x += w;
    }
    let i = at?;
    if chars[i].is_whitespace() {
        return None;
    }
    let mut a = i;
    while a > 0 && !chars[a - 1].is_whitespace() {
        a -= 1;
    }
    let mut b = i;
    while b + 1 < chars.len() && !chars[b + 1].is_whitespace() {
        b += 1;
    }
    let antes = chars[..a].iter().all(|c| c.is_whitespace());
    let despues = chars[b + 1..].iter().all(|c| c.is_whitespace());
    Some((chars[a..=b].iter().collect(), antes, despues))
}

fn ultima(text: &str, skip: usize) -> Option<String> {
    let t: String = text.chars().skip(skip).collect();
    t.trim_end().rsplit(char::is_whitespace).next().filter(|s| !s.is_empty()).map(str::to_string)
}

fn primera(text: &str, skip: usize) -> Option<String> {
    let t: String = text.chars().skip(skip).collect();
    t.split_whitespace().next().map(str::to_string)
}

/// Abre la URL en el navegador de siempre. Solo http(s): nada de `file://` ni otros esquemas.
pub fn abrir(url: &str) -> bool {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return false;
    }
    #[cfg(windows)]
    let mut cmd = {
        // `start` pasa por cmd, que se come los `&` de la URL; este no.
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok()
}

/// El dominio, para el aviso: «abriendo claude.ai».
pub fn dominio(url: &str) -> &str {
    let sin = url.split("://").nth(1).unwrap_or(url);
    sin.split(['/', '?', '#']).next().unwrap_or(sin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::select::Row;
    use ratatui::layout::Rect;

    fn vista(filas: &[(&str, usize, bool)]) -> View {
        View {
            area: Rect::new(0, 0, 40, filas.len() as u16),
            rows: filas.iter().map(|(t, s, c)| Row { text: t.to_string(), skip: *s, cont: *c }).collect(),
            first: 0,
        }
    }

    #[test]
    fn saca_urls_de_markdown_y_sueltas() {
        let u = urls("Mira [el doc](https://claude.ai/code/artifact/abc-123). Y <https://a.b/c?x=1&y=2>, o http://x.y/z.");
        assert_eq!(u, vec!["https://claude.ai/code/artifact/abc-123", "https://a.b/c?x=1&y=2", "http://x.y/z"]);
    }

    #[test]
    fn clic_en_una_url_entera() {
        let u = urls("está en https://iris.knarvaez.com/tienda hoy");
        let v = vista(&[("  está en https://iris.knarvaez.com/tienda hoy", 2, false)]);
        assert_eq!(bajo(&v, (0, 15), &u).as_deref(), Some("https://iris.knarvaez.com/tienda"));
        assert_eq!(bajo(&v, (0, 4), &u), None);
    }

    #[test]
    fn clic_en_cualquier_pedazo_de_una_url_partida() {
        let url = "https://claude.ai/code/artifact/dfa3e55a-dd1c-4525-8cbf-84ebaa24e0cb";
        let u = urls(&format!("El documento está aquí: [Plan]({url})"));
        let v = vista(&[
            ("  El documento está aquí: Plan", 2, false),
            ("  (https://claude.ai/code/artifact/dfa3e5", 2, true),
            ("  5a-dd1c-4525-8cbf-84ebaa24e0cb)", 2, true),
        ]);
        assert_eq!(bajo(&v, (1, 10), &u).as_deref(), Some(url));
        assert_eq!(bajo(&v, (2, 6), &u).as_deref(), Some(url));
    }

    #[test]
    fn markdown_en_crudo_como_en_el_subtitulo_del_cine() {
        let url = "https://claude.ai/code/artifact/dfa3e55a-dd1c-4525-8cbf-84ebaa24e0cb";
        let u = urls(&format!("El documento está aquí: [Plan técnico]({url})"));
        let v = vista(&[
            ("      El documento está aquí: [Plan técnico](https://claude.ai/code/", 6, false),
            ("             artifact/dfa3e55a-dd1c-4525-8cbf-84ebaa24e0cb)", 13, true),
        ]);
        assert_eq!(bajo(&v, (0, 50), &u).as_deref(), Some(url));
        assert_eq!(bajo(&v, (0, 40), &u).as_deref(), Some(url));
        assert_eq!(bajo(&v, (1, 20), &u).as_deref(), Some(url));
        assert_eq!(bajo(&v, (0, 9), &u), None);
    }

    #[test]
    fn una_url_partida_en_tres_filas() {
        let url = "https://example.com/aaaaaaaaaaaaaaaaaaaa/bbbbbbbbbbbbbbbbbbbb/cccc";
        let u = urls(url);
        let v = vista(&[("  https://example.com/aaaaaaaaaaaaaaaaaa", 2, false), ("  aa/bbbbbbbbbbbbbb", 2, true), ("  bbbbbb/cccc y sigue", 2, true)]);
        assert_eq!(bajo(&v, (2, 4), &u).as_deref(), Some(url));
        assert_eq!(bajo(&v, (1, 4), &u).as_deref(), Some(url));
    }

    #[test]
    fn una_palabra_que_tambien_esta_en_una_url_no_abre_nada() {
        let u = urls("ver https://iris.knarvaez.com/tienda");
        let v = vista(&[("  la tienda abre pronto, ver", 2, false), ("  https://iris.knarvaez.com/tienda", 2, true)]);
        assert_eq!(bajo(&v, (0, 5), &u), None);
        // La palabra de después de una URL partida tampoco.
        let v = vista(&[("  ver https://iris.knarvaez.com/tienda", 2, false), ("  sigue aquí", 2, true)]);
        assert_eq!(bajo(&v, (1, 3), &u), None);
    }

    #[test]
    fn el_dominio_para_el_aviso() {
        assert_eq!(dominio("https://claude.ai/code/x?y=1"), "claude.ai");
    }
}
