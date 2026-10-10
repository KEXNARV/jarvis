//! SALIDA: lo pintado en la rejilla, a la terminal. En braille cada celda junta 2×4 puntos y
//! toma el color del más brillante; en imagen (Sixel) cada punto es un círculo con su color.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::color::rgb;
use super::rejilla::Grid;
use super::Core;

impl Core {
    pub fn draw(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 6 || area.height < 3 {
            return;
        }
        let (cw, ch) = (area.width as usize, area.height as usize);
        let mut g = Grid::new(cw, ch);
        self.paint(&mut g);
        let pal = self.palette();
        const BITS: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
        for cy in 0..ch {
            for cx in 0..cw {
                // La celda: los bits de sus 8 puntos y el color del más brillante.
                let (mut bits, mut best) = (0u8, 0u8);
                for dy in 0..4 {
                    for dx in 0..2 {
                        let d = g.dots[(cy * 4 + dy) * g.dw + cx * 2 + dx];
                        if d & 3 != 0 {
                            bits |= BITS[dy][dx];
                            if (d & 3, d >> 2) > (best & 3, best >> 2) {
                                best = d;
                            }
                        }
                    }
                }
                if bits == 0 {
                    continue;
                }
                let ch = char::from_u32(0x2800 + bits as u32).unwrap_or(' ');
                let pos = (area.x + cx as u16, area.y + cy as u16);
                let c = pal[(best >> 2) as usize][(best & 3) as usize];
                buf[pos].set_char(ch).set_fg(rgb(c));
            }
        }
    }

    /// El núcleo como imagen Sixel de `w`×`h` píxeles, con puntos redondos cada `sp` píxeles y
    /// fondo transparente: la misma estética de puntos que el braille, más fina y con un color
    /// por punto en vez de uno por celda.
    pub fn sixel(&self, w: usize, h: usize, sp: usize) -> String {
        let (img, colors) = self.pixels(w, h, sp);
        crate::sixel::encode(&img, w, h, &colors)
    }

    /// La imagen de puntos sin codificar: un índice de color por píxel (0 = transparente).
    pub fn pixels(&self, w: usize, h: usize, sp: usize) -> (Vec<u8>, Vec<[f64; 3]>) {
        let sp = sp.max(2);
        let (dw, dh) = ((w / sp).max(8), (h / sp).max(8));
        let (dots, colors) = self.dots(dw, dh);
        let mut img = vec![0u8; w * h];
        // Casi media separación de radio: círculos que se ven como puntos y no como cuadraditos.
        let r = sp as f64 * 0.46;
        let offs: Vec<(isize, isize)> = {
            let n = r.ceil() as isize;
            (-n..=n).flat_map(|y| (-n..=n).map(move |x| (x, y))).filter(|(x, y)| ((*x * *x + *y * *y) as f64) <= r * r).collect()
        };
        for j in 0..dh {
            for i in 0..dw {
                let idx = dots[j * dw + i];
                if idx == 0 {
                    continue;
                }
                let (cx, cy) = ((i * sp + sp / 2) as isize, (j * sp + sp / 2) as isize);
                for (ox, oy) in &offs {
                    let (x, y) = (cx + ox, cy + oy);
                    if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                        img[y as usize * w + x as usize] = idx;
                    }
                }
            }
        }
        (img, colors)
    }

    /// Los puntos sin dibujar: una rejilla de `dw`×`dh` con un índice de color por punto
    /// (0 = apagado). La página de Iris los pinta ella misma, que es mucho más barato que
    /// recorrer una imagen entera en cada cuadro.
    pub fn dots(&self, dw: usize, dh: usize) -> (Vec<u8>, Vec<[f64; 3]>) {
        // Bloques de ~3×3 puntos: lo de atrás queda tapado igual que en braille.
        let mut g = Grid::with_dots(dw, dh, 3, 3);
        self.paint(&mut g);
        let pal = self.palette();
        // Índice 0 = transparente; 1 + tinta·3 + (tono-1) para los demás.
        let dots = g.dots.iter().map(|&d| if d & 3 == 0 { 0 } else { 1 + (d >> 2) * 3 + (d & 3) - 1 }).collect();
        let colors = pal.iter().flat_map(|ink| (1..4).map(move |tone| ink[tone])).collect();
        (dots, colors)
    }
}
