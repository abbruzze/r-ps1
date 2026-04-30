use fontdue::{Font, FontSettings, Metrics};
use std::collections::HashMap;

static FONT: &[u8] = include_bytes!("../../resources/Commodore Rounded v1.2.ttf");


// ---------------------------------------------------------------------------
// Glyph cache entry
// ---------------------------------------------------------------------------
#[derive(Clone)]
struct CachedGlyph {
    metrics: Metrics,
    bitmap: Vec<u8>,
}

// ---------------------------------------------------------------------------
// TextRenderer
// ---------------------------------------------------------------------------
pub struct TextRenderer {
    font: Font,
    /// cache: (char, size_bits) -> rasterizzato
    cache: HashMap<(char, u32), CachedGlyph>,
}

impl TextRenderer {
    pub fn new() -> Self {
        let font = Font::from_bytes(FONT, FontSettings::default())
            .expect("Error while loading font");
        Self {
            font,
            cache: HashMap::new(),
        }
    }

    fn rasterize(&mut self, ch: char, size: f32) -> CachedGlyph {
        // La dimensione viene quantizzata a 0.5px per evitare cache miss eccessivi
        let size_key = (size * 2.0).round() as u32;
        let key = (ch, size_key);

        if !self.cache.contains_key(&key) {
            let (metrics, bitmap) = self.font.rasterize(ch, size);
            self.cache.insert(key, CachedGlyph { metrics, bitmap });
        }

        self.cache[&key].clone()
    }

    pub fn measure(&mut self, text: &str, size: f32) -> (u32, u32) {
        let mut width = 0u32;
        let mut max_height = 0u32;
        for ch in text.chars() {
            let g = self.rasterize(ch, size);
            width += g.metrics.advance_width.ceil() as u32;
            max_height = max_height.max(g.metrics.height as u32);
        }
        (width, max_height)
    }

    // -----------------------------------------------------------------------
    // Disegna una singola riga di testo nel framebuffer RGBA8
    //
    // Parametri:
    //   frame        — slice mutabile restituito da `pixels.frame_mut()`
    //   screen_width — larghezza in pixel dello schermo (per calcolare l'offset)
    //   text         — stringa da disegnare
    //   x, y         — coordinata top-left del testo
    //   size         — dimensione font in punti
    //   color        — [R, G, B, A] colore del testo (A è opacità 0..255)
    // -----------------------------------------------------------------------
    pub fn draw_text(
        &mut self,
        frame: &mut [u8],
        screen_width: u32,
        text: &str,
        x: i32,
        y: i32,
        size: f32,
        color: [u8; 4],
    ) {
        // baseline_y: riga assoluta nel framebuffer della baseline condivisa.
        // ascender = max(height + ymin) tra tutti i glifi = quanto sale il glifo
        // piu alto rispetto alla baseline.
        // Cosi y corrisponde al top del glifo piu alto, e tutti i glifi
        // condividono la stessa baseline → nessun disallineamento verticale.
        let ascender = text
            .chars()
            .map(|ch| {
                let g = self.rasterize(ch, size);
                g.metrics.height as i32 + g.metrics.ymin
            })
            .max()
            .unwrap_or(size as i32);

        let baseline_y = y + ascender;

        let mut cursor_x = x;

        for ch in text.chars() {
            let g = self.rasterize(ch, size);
            let m = &g.metrics;

            // top del glifo = baseline - (height + ymin)
            let glyph_y = baseline_y - m.height as i32 - m.ymin;

            for py in 0..m.height {
                for px in 0..m.width {
                    let alpha_glyph = g.bitmap[py * m.width + px];
                    if alpha_glyph == 0 {
                        continue;
                    }

                    let fx = cursor_x + px as i32 + m.xmin;
                    let fy = glyph_y + py as i32;

                    // Clipping
                    if fx < 0 || fy < 0 {
                        continue;
                    }
                    let fx = fx as u32;
                    let fy = fy as u32;
                    if fx >= screen_width {
                        continue;
                    }

                    let idx = ((fy * screen_width + fx) * 4) as usize;
                    if idx + 3 >= frame.len() {
                        continue;
                    }

                    // Alpha blending: src over dst
                    // alpha finale = alpha_glyph * color[3] / 255
                    let src_a = (alpha_glyph as u32 * color[3] as u32) / 255;
                    let dst_a = 255u32 - src_a;

                    for i in 0..3usize {
                        frame[idx + i] = ((color[i] as u32 * src_a
                            + frame[idx + i] as u32 * dst_a)
                            / 255) as u8;
                    }
                    frame[idx + 3] = 255;
                }
            }

            cursor_x += m.advance_width.ceil() as i32;
        }
    }

    // -----------------------------------------------------------------------
    // Disegna testo multi-riga (separa su '\n')
    // line_gap: pixel extra tra le righe (oltre all'altezza del font)
    // -----------------------------------------------------------------------
    pub fn draw_text_multiline(
        &mut self,
        frame: &mut [u8],
        screen_width: u32,
        text: &str,
        x: i32,
        y: i32,
        size: f32,
        line_gap: i32,
        color: [u8; 4],
    ) {
        let line_height = size as i32 + line_gap;
        for (i, line) in text.lines().enumerate() {
            self.draw_text(
                frame,
                screen_width,
                line,
                x,
                y + i as i32 * line_height,
                size,
                color,
            );
        }
    }

    // -----------------------------------------------------------------------
    // Disegna testo con sfondo colorato (utile per HUD/debug overlay)
    // padding: pixel di margine attorno al testo
    // -----------------------------------------------------------------------
    pub fn draw_text_with_background(
        &mut self,
        frame: &mut [u8],
        screen_width: u32,
        screen_height: u32,
        text: &str,
        x: i32,
        y: i32,
        size: f32,
        color: [u8; 4],
        bg_color: [u8; 4],
        padding: i32,
    ) {
        let (tw, th) = self.measure(text, size);

        // Disegna il rettangolo di sfondo
        let bx = (x - padding).max(0) as u32;
        let by = (y - padding).max(0) as u32;
        let bw = tw + (padding * 2) as u32;
        let bh = th + (padding * 2) as u32;

        fill_rect(frame, screen_width, screen_height, bx, by, bw, bh, bg_color);

        // Disegna il testo sopra
        self.draw_text(frame, screen_width, text, x, y, size, color);
    }

    // -----------------------------------------------------------------------
    // Testo allineato a destra (utile per contatori, FPS...)
    // -----------------------------------------------------------------------
    pub fn draw_text_right(
        &mut self,
        frame: &mut [u8],
        screen_width: u32,
        text: &str,
        right_x: i32,
        y: i32,
        size: f32,
        color: [u8; 4],
    ) {
        let (tw, _) = self.measure(text, size);
        let x = right_x - tw as i32;
        self.draw_text(frame, screen_width, text, x, y, size, color);
    }

    // -----------------------------------------------------------------------
    // Svuota la cache dei glifi (utile se cambi font o liberi memoria)
    // -----------------------------------------------------------------------
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

// ---------------------------------------------------------------------------
// Utility: riempie un rettangolo nel framebuffer con alpha blending
// ---------------------------------------------------------------------------
pub fn fill_rect(
    frame: &mut [u8],
    screen_width: u32,
    screen_height: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: [u8; 4],
) {
    let src_a = color[3] as u32;
    let dst_a = 255 - src_a;

    for py in y..(y + h).min(screen_height) {
        for px in x..(x + w).min(screen_width) {
            let idx = ((py * screen_width + px) * 4) as usize;
            if idx + 3 >= frame.len() {
                continue;
            }
            for i in 0..3usize {
                frame[idx + i] =
                    ((color[i] as u32 * src_a + frame[idx + i] as u32 * dst_a) / 255) as u8;
            }
            frame[idx + 3] = 255;
        }
    }
}