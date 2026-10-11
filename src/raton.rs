//! El mouse: rueda, clics en adjuntos, miniaturas y enlaces, y arrastrar para copiar del chat.

use crate::*;

pub(crate) fn on_mouse(app: &mut App, m: crossterm::event::MouseEvent) {
    let at = (m.column, m.row);
    let area = app.view.borrow().area;
    let inside = area.contains(ratatui::layout::Position::new(m.column, m.row));
    let dragging = app.sel.as_ref().is_some_and(|s| s.pointer.is_some());
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let hit = app.file_targets.borrow().iter().find(|(r, _)| r.contains(ratatui::layout::Position::new(m.column, m.row))).map(|(_, a)| a.clone());
            if let Some(a) = hit {
                app.sel = None;
                let msg = if adjunto::copiar(&a) {
                    app.nucleo.fire(Gesto::Copy);
                    "archivo copiado · pégalo donde quieras".to_string()
                } else {
                    "no pude copiar el archivo al portapapeles".to_string()
                };
                app.flash = Some((msg, Instant::now()));
                return;
            }
            app.sel = inside.then(|| {
                let p = app.view.borrow().locate(at);
                select::Selection { anchor: p, head: p, pointer: Some(at) }
            });
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(s) = &mut app.sel {
                s.pointer = Some(at);
                s.head = app.view.borrow().locate(at);
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            let Some(s) = &mut app.sel else { return };
            s.pointer = None;
            if s.is_empty() {
                app.sel = None;
                // Un clic sin arrastrar sobre una URL la abre en el navegador.
                abrir_enlace(app, at);
                return;
            }
            // Queda resaltado hasta el próximo clic o tecla, para ver qué se copió.
            let text = s.text(&app.view.borrow());
            copied(app, &text);
        }
        // Con la rueda se puede seguir estirando la selección más allá de lo que se ve.
        MouseEventKind::ScrollUp if app.reading.get() => scroll(app, true, 3),
        MouseEventKind::ScrollDown if app.reading.get() => scroll(app, false, 3),
        MouseEventKind::ScrollUp if inside || dragging => {
            app.scroll = (app.scroll + 3).min(app.view.borrow().max_scroll());
        }
        MouseEventKind::ScrollDown if inside || dragging => app.scroll = app.scroll.saturating_sub(3),
        _ => {}
    }
}

/// Cada cuadro, mientras se arrastra: la punta sigue al puntero sobre el texto que ahora
/// está debajo, y arrastrar por encima o por debajo del chat lo desplaza solo.
pub(crate) fn follow_drag(app: &mut App) {
    let Some(p) = app.sel.as_ref().and_then(|s| s.pointer) else { return };
    let area = app.view.borrow().area;
    if p.1 < area.y {
        app.scroll = (app.scroll + 1).min(app.view.borrow().max_scroll());
    } else if p.1 >= area.bottom() {
        app.scroll = app.scroll.saturating_sub(1);
    }
    let head = app.view.borrow().locate(p);
    if let Some(s) = &mut app.sel {
        s.head = head;
    }
}

/// `/copy`: la última respuesta entera, con su markdown.
pub(crate) fn copy_last(app: &mut App) {
    match app.messages.iter().rev().find(|m| m.role == Role::Assistant) {
        Some(m) => {
            let text = m.text.clone();
            copied(app, &text);
        }
        None => app.flash = Some(("todavía no hay respuesta que copiar".into(), Instant::now())),
    }
}

pub(crate) fn copied(app: &mut App, text: &str) {
    let n = text.chars().count();
    let msg = if text.is_empty() {
        "nada que copiar ahí".to_string()
    } else if select::copy(text) {
        app.nucleo.fire(Gesto::Copy);
        format!("copiado · {n} caracteres")
    } else {
        "no pude copiar al portapapeles".to_string()
    };
    app.flash = Some((msg, Instant::now()));
}

/// Si bajo el puntero hay una URL de la conversación, la abre en el navegador.
fn abrir_enlace(app: &mut App, at: (u16, u16)) {
    let urls: Vec<String> = app.messages.iter().flat_map(|m| enlace::urls(&m.text)).collect();
    if urls.is_empty() {
        return;
    }
    let view = app.view.borrow();
    if !view.area.contains(ratatui::layout::Position::new(at.0, at.1)) {
        return;
    }
    let Some(url) = enlace::bajo(&view, view.locate(at), &urls) else { return };
    drop(view);
    let msg = if enlace::abrir(&url) {
        format!("abriendo {}", enlace::dominio(&url))
    } else {
        "no pude abrir el enlace".to_string()
    };
    app.flash = Some((msg, Instant::now()));
}
