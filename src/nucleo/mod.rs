//! NÚCLEO: el panel de arriba a la derecha, donde vive el buddy. Muestra qué está haciendo
//! Iris con la forma, el movimiento y una mirada que va hacia donde pasan las cosas.
//!
//! El diseño se hizo en `docs/blob-hibrido.html`; esto es su versión para la terminal. Se
//! dibuja sobre una rejilla braille propia (2×4 puntos por celda) y no con el `Canvas` de
//! ratatui porque ahí cada celda toma el color del último punto dibujado: acá se queda con el
//! del punto más brillante, que es lo que hace legibles las capas.
//!
//! El núcleo solo orquesta: lleva el reloj y el estado que se ve (con su mínimo, para que no
//! parpadee), le pasa al buddy las señales, los eventos y los subagentes, y saca lo que
//! dibuja en braille o en Sixel. Lo que se ve lo decide cada buddy (`buddies`) con sus piezas
//! (`piezas`).

use ratatui::style::Color;

mod azar;
mod buddies;
mod color;
mod contexto;
mod estado;
mod piezas;
mod rejilla;
mod salida;

pub use estado::{tool_state, Event, Signals, State};

use buddies::baymax::{Baymax, Cara};
use buddies::original::Original;
use buddies::Buddy;
use color::rgb;
use contexto::{Aviso, Contexto};
use piezas::cuerpo::Blob;
use rejilla::Grid;

pub struct Core {
    ctx: Contexto,
    fired: Vec<Event>,
    /// Los buddies que pueden vivir en el núcleo, todos andando a la vez: así cambiar de uno a
    /// otro (`/buddy`, incluso en la vista previa) no los reinicia. El primero es el original;
    /// el segundo, Baymax. Los hijos tienen solo el primero.
    buddies: Vec<Box<dyn Buddy>>,
}

/// Qué piezas del buddy original se dibujan, un bit por pieza (`CAPA_*` en
/// `buddies::original`); `u32::MAX` es todas. Para la página de Iris, que lo muestra desarmado.
#[allow(dead_code)]
pub fn capas(mascara: u32) {
    buddies::original::CAPAS.store(mascara, std::sync::atomic::Ordering::Relaxed);
}

impl Core {
    pub fn new() -> Self {
        let baymax: Baymax = Original::nuevo(Cara::new());
        Core::con(vec![Box::new(Original::nuevo(Blob::new())), Box::new(baymax)], 0.0, State::Booting)
    }

    fn con(buddies: Vec<Box<dyn Buddy>>, t: f64, estado: State) -> Self {
        Core { ctx: Contexto { t, estado, desde: t, sig: Signals::default(), mira: None }, fired: vec![], buddies }
    }

    /// Un hijo: ya despierto en `state`, sin arranque ni motas.
    fn mini(state: State, seed: u64) -> Self {
        // Cada hijo con su fase, para que no respiren ni parpadeen todos a la vez.
        Core::con(vec![Box::new(Original::mini(state, seed))], 3.7 * seed as f64, state)
    }

    /// El que se ve: Baymax si su paleta está puesta (y no es un hijo).
    fn activo(&self) -> &dyn Buddy {
        let i = if self.buddies.len() > 1 && crate::theme::baymax() { 1 } else { 0 };
        &*self.buddies[i]
    }

    fn avisar(&mut self, aviso: Aviso) {
        for b in &mut self.buddies {
            b.aviso(aviso, &self.ctx);
        }
    }

    /// El estado que se ve, que puede ir un poco detrás del pedido (ver `State::dwell`).
    pub fn state(&self) -> State {
        self.ctx.estado
    }

    /// El color actual, con las transiciones y el rojo de los errores.
    pub fn color(&self) -> Color {
        rgb(self.activo().color())
    }

    /// Reinicia la animación de arranque (al relanzar el motor).
    pub fn reboot(&mut self) {
        self.kids_clear();
        self.switch(State::Booting);
    }

    /// Nace un subagente: brota del cuerpo hacia su lugar en la órbita.
    pub fn kid_born(&mut self, id: u64) {
        self.avisar(Aviso::Nace(id));
    }

    /// El hijo hizo algo (pidió una herramienta o le llegó su resultado): una partícula sube.
    pub fn kid_pulse(&mut self, id: u64, error: bool) {
        self.avisar(Aviso::Pulso(id, error));
    }

    /// Terminó el subagente: si salió bien vuelve al cuerpo; si no, se apaga en rojo.
    pub fn kid_end(&mut self, id: u64, ok: bool) {
        self.avisar(Aviso::Fin(id, ok));
    }

    /// El motor se fue: los hijos se van con él, sin ceremonia.
    pub fn kids_clear(&mut self) {
        self.avisar(Aviso::Fuera);
    }

    /// Cuántos hijos hay (vivos o despidiéndose).
    #[cfg(test)]
    pub fn kid_count(&self) -> usize {
        self.activo().hijos()
    }

    /// Los eventos disparados desde la última vez, para avisarle al teclado.
    pub fn take_fired(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.fired)
    }

    pub fn fire(&mut self, ev: Event) {
        self.fired.push(ev);
        self.avisar(Aviso::Evento(ev));
    }

    fn switch(&mut self, to: State) {
        let from = self.ctx.estado;
        self.ctx.estado = to;
        self.ctx.desde = self.ctx.t;
        for b in &mut self.buddies {
            b.cambio(from, to, &self.ctx);
        }
    }

    /// Avanza la animación `dt` segundos hacia el estado `want`.
    pub fn step(&mut self, dt: f64, want: State, sig: &Signals) {
        let dt = dt.clamp(0.0, 0.1);
        self.ctx.t += dt;
        self.ctx.sig = sig.clone();
        let shown = self.ctx.estado;
        let shown_for = self.ctx.t - self.ctx.desde;
        if want != shown && (want.urgent() || shown.urgent() || shown_for >= shown.dwell()) {
            self.switch(want);
        }
        for b in &mut self.buddies {
            b.step(&self.ctx, dt);
        }
    }

    /// Si está puesto, la mirada va hacia ahí (−1..1) en vez de lo que diga el estado.
    fn mirar_hacia(&mut self, mira: Option<[f64; 2]>) {
        self.ctx.mira = mira;
    }

    /// Cuánto error lleva encima (0..1).
    fn alarma(&self) -> f64 {
        self.activo().alarma()
    }

    /// Su color sin pasar a la terminal: para la tinta de un hijo.
    fn tinta(&self) -> [f64; 3] {
        self.activo().color()
    }

    fn paint(&self, g: &mut Grid) {
        self.activo().paint(g, &self.ctx);
    }

    fn palette(&self) -> Vec<[[f64; 3]; 4]> {
        self.activo().paleta()
    }
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
