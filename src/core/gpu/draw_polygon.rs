//! Structure representing texture attributes for polygons.
//!
//! This struct holds information regarding texture mapping, such as CLUT (Color Lookup Table) coordinates,
//! page coordinates for textures, semi-transparency mode, and texture depth.
use crate::core::gpu::gp0::DITHER_TABLE;
use crate::core::gpu::timings::GPUTimings;
use crate::core::gpu::{Color, Gp0State, SemiTransparency, TextureDepth, Vertex, GPU};
use crate::core::interrupt::IrqHandler;
use std::cmp;
use tracing::debug;

#[derive(Debug,Clone,Default)]
struct PolygonTexture {
    clut_x: u32,
    clut_y: u32,
    page_base_x: u8,
    page_base_y: u8,
    semi_transparency: SemiTransparency,
    texture_depth: TextureDepth,
}

#[derive(Debug,Clone,Default,Copy)]
struct UV {
    u: i32,
    v: i32,
}

#[derive(Debug,Clone,Default)]
struct Polygon {
    vertex: Vec<(Vertex,Color,UV)>,
    texture: Option<PolygonTexture>,
}

impl GPU {
    /*
    GPU Render Polygon Commands
    When the upper 3 bits of the first GP0 command are set to 1 (001), then the command can be decoded using the following bitfield:

     bit number   value   meaning
      31-29        001    polygon render
        28         1/0    gouraud / flat shading
        27         1/0    4 / 3 vertices
        26         1/0    textured / untextured
        25         1/0    semi-transparent / opaque
        24         1/0    raw texture / modulation
       23-0        rgb    first color value.
    Subsequent data sent to GP0 to complete this command will be the vertex data for the command. The meaning and count of these words will be altered by the initial flags sent in the first command.

    If doing flat rendering, no further color will be sent. If doing gouraud shading, there will be one more color per vertex sent, and the initial color will be the one for vertex 0.

    If doing textured rendering, each vertex sent will also have a U/V texture coordinate attached to it, as well as a CLUT index.

    So each vertex data can be seen as the following set of words:

    Color      xxBBGGRR               - optional, only present for gouraud shading
    Vertex     YYYYXXXX               - required, two signed 16 bits values
    UV         ClutVVUU or PageVVUU   - optional, only present for textured polygons
    The upper 16 bits of the first two UV words contain extra information. The first word holds the Clut index. The second word contains texture page information. Any further clut/page bits should be set to 0.
    Notes
    Polygons are displayed up to \<excluding> their lower-right coordinates.
    Quads are internally processed as two triangles, the first consisting of vertices 1,2,3, and the second of vertices 2,3,4. This is an important detail, as splitting the quad into triangles affects the way colours are interpolated.
    Within the triangle, the ordering of the vertices doesn't matter on the GPU side (a front-back check, based on clockwise or anti-clockwise ordering, can be implemented at the GTE side).
    Dither enable (in Texpage command) affects ONLY polygons that do use gouraud shading or modulation.
     */
    pub(super) fn operation_polygon_rendering(&mut self,cmd:u32,_irq_handler:&mut IrqHandler) -> usize {
        match self.gp0state {
            Gp0State::WaitingCommandParameters(operation, None) => {
                let is_gouraud = (cmd & (1 << 28)) != 0;
                let is_4_vertex = (cmd & (1 << 27)) != 0;
                let is_textured = (cmd & (1 << 26)) != 0;

                let vertex = if is_4_vertex { 4 } else { 3 };
                let mut expected_data = vertex;

                if is_gouraud {
                    expected_data += vertex - 1; // vertex - 1 colors, the first one is encoded in the command
                }
                if is_textured {
                    expected_data += vertex;
                }

                self.gp0state = Gp0State::WaitingCommandParameters(operation, Some(expected_data));
                0
            }
            Gp0State::WaitingCommandParameters(_, Some(_)) => {
                let cmd = self.cmd_fifo.pop().unwrap();
                let is_gouraud = (cmd & (1 << 28)) != 0;
                let is_textured = (cmd & (1 << 26)) != 0;
                let semi_transparent = (cmd & (1 << 25)) != 0;
                let raw_texture = (cmd & (1 << 24)) != 0;
                let shading_color = Color::from_u32(cmd);

                let mut polygon = Polygon::default();
                if is_textured {
                    polygon.texture = Some(PolygonTexture::default());
                }

                let mut word_index = 0usize;

                while !self.cmd_fifo.is_empty() {
                    let (mut vertex,color) = if is_gouraud {
                        let color = if word_index == 0 {
                            shading_color.clone()
                        }
                        else {
                            Color::from_u32(self.cmd_fifo.pop().unwrap())
                        };
                        (Vertex::from_command_parameter(self.cmd_fifo.pop().unwrap()),color)
                    }
                    else {
                        (Vertex::from_command_parameter(self.cmd_fifo.pop().unwrap()),shading_color.clone())
                    };
                    // add X,Y, drawing offset
                    vertex.add_offset(self.drawing_area.x_offset,self.drawing_area.y_offset);

                    let mut uv = UV::default();
                    if is_textured {
                        let word = self.cmd_fifo.pop().unwrap();
                        let base_u = word as u8;
                        let base_v = (word >> 8) as u8;
                        let texture = polygon.texture.as_mut().unwrap();
                        uv.u = base_u as i32;
                        uv.v = base_v as i32;
                        match word_index {
                            0 => {
                                let clut = word >> 16;
                                let clut_x = clut & 0x3F; // 0-5    X coordinate X/16
                                let clut_y = (clut >> 6) & 0x1FF; // 6-14   Y coordinate 0-511
                                texture.clut_x = clut_x;
                                texture.clut_y = clut_y;
                            }
                            1 => {
                                /*
                                Texpage Attribute (Parameter for Textured-Polygons commands)
                                  0-8    Same as GP0(E1h).Bit0-8 (see there)
                                  9-10   Unused (does NOT change GP0(E1h).Bit9-10)
                                  11     Same as GP0(E1h).Bit11  (see there)
                                  12-13  Unused (does NOT change GP0(E1h).Bit12-13)
                                  14-15  Unused (should be 0)

                                GP0(E1h) - Draw Mode setting (aka "Texpage")
                                  0-3   Texture page X Base   (N*64) (ie. in 64-halfword steps)    ;GPUSTAT.0-3
                                  4     Texture page Y Base 1 (N*256) (ie. 0, 256, 512 or 768)     ;GPUSTAT.4
                                  5-6   Semi-transparency     (0=B/2+F/2, 1=B+F, 2=B-F, 3=B+F/4)   ;GPUSTAT.5-6
                                  7-8   Texture page colors   (0=4bit, 1=8bit, 2=15bit, 3=Reserved);GPUSTAT.7-8
                                 */
                                let page = word >> 16;
                                let page_base_x = (page & 0xF) as u8;
                                let page_base_y = ((page >> 4) & 0x1) as u8;
                                let semi_transparency = SemiTransparency::from_command(page);
                                let texture_depth = match (page >> 7) & 3 {
                                    0 => TextureDepth::T4Bit,
                                    1 => TextureDepth::T8Bit,
                                    2 => TextureDepth::T15Bit,
                                    3 => TextureDepth::Reserved,
                                    _ => unreachable!()
                                };
                                texture.page_base_x = page_base_x;
                                texture.page_base_y = page_base_y;
                                texture.texture_depth = texture_depth;
                                texture.semi_transparency = semi_transparency;
                                // update gloabl texture info
                                self.texture.page_base_x = page_base_x;
                                self.texture.page_base_y = page_base_y;
                                self.texture.depth = texture_depth;
                                self.semi_transparency = semi_transparency;
                                //self.dithering = is_gouraud || (is_textured && !raw_texture);
                            }
                            _ => {}
                        }
                    }
                    polygon.vertex.push((vertex,color,uv));
                    word_index += 1;
                }

                let pixels = self.draw_polygon(&polygon,is_gouraud,is_textured,semi_transparent,raw_texture);
                self.gp0state = Gp0State::WaitingCommand;
                pixels
            }
            _ => {
                0
            }
        }
    }

    fn draw_polygon(&mut self, polygon:&Polygon,is_gouraud:bool,is_textured:bool,is_semi_transparent:bool,is_raw_texture:bool) -> usize {
        debug!("Drawing polygon: {:?} gouraud={is_gouraud} textured={is_textured} semi_transparent={is_semi_transparent} raw_texture={is_raw_texture}",polygon);
        if polygon.vertex.len() < 3 {
            return 0;
        }
        let mut pixels = self.draw_triangle::<0>(polygon, is_gouraud, is_textured, is_semi_transparent, is_raw_texture);
        if polygon.vertex.len() == 4 {
            pixels += self.draw_triangle::<1>(polygon, is_gouraud, is_textured, is_semi_transparent, is_raw_texture);
        }
        GPUTimings::triangle(pixels,is_gouraud,is_semi_transparent,is_textured)
    }

    #[inline(always)]
    fn edge_function(a: &Vertex, b: &Vertex, c: &Vertex) -> i32 {
        (b.x as i32 - a.x as i32) * (c.y as i32 - a.y as i32) - (b.y as i32 - a.y as i32) * (c.x as i32 - a.x as i32)
    }

    #[inline(always)]
    fn is_top_left(a: &Vertex, b: &Vertex) -> bool {
        let dy = b.y - a.y;
        let dx = b.x - a.x;

        (dy < 0) || (dy == 0 && dx > 0)
    }

    /*
        Optimized with AI using fixed point arithmetic.

        This function rasterizes one triangle of a polygon. For quads, it is called twice with a
        different OFFSET so that the polygon is split into two triangles.

        The implementation first selects the three vertices, computes the signed triangle area with
        edge_function, and normalizes the winding order so that the area is positive. Degenerate
        triangles with zero area are discarded immediately.

        Rasterization is performed with edge functions. Each edge is classified with the top-left
        rule and receives a small bias, so pixels lying exactly on shared edges are handled
        consistently and adjacent triangles do not draw the same pixel twice.

        A clipped bounding box is computed in VRAM coordinates, limiting the scan area to the
        1024x512 framebuffer. The function then scans the bounding box row by row. For each row,
        the edge values are initialized at the left side of the box and then incrementally updated
        while moving along the X axis, avoiding repeated full edge-function evaluations per pixel.

        For Gouraud shaded or textured triangles, barycentric weights are computed from the edge
        values. The reciprocal of the triangle area is stored in fixed point form, allowing color
        components and texture coordinates to be interpolated with integer arithmetic instead of
        floating point operations. Per-pixel X steps for RGB and UV values are precomputed once and
        then accumulated inside the inner loop.

        If texturing is enabled, interpolated UV coordinates are adjusted through the current texture
        window settings, then used to fetch a texel from VRAM according to the texture depth
        (4-bit CLUT, 8-bit CLUT, or 15-bit direct color). A zero texel is treated as transparent.
        Non-transparent texels either replace the current color in raw texture mode or are modulated
        with the shaded color.

        Dithering is applied only when enabled by GPU state and when required by Gouraud shading or
        texture modulation. Semi-transparency is applied according to the polygon flag and, for
        textured polygons, only when the texture pixel allows it through bit 15.

        Each visible pixel is finally clipped against the drawing area, converted to RGB555, and
        written to VRAM through draw_pixel_offset so mask-bit and semi-transparency rules are honored.
        The function returns the number of pixels actually drawn, which is later used for GPU timing.
         */
    fn draw_triangle<const OFFSET: usize>(&mut self, polygon: &Polygon, is_gouraud: bool, is_textured: bool, is_semi_transparent: bool, is_raw_texture: bool) -> usize {
        let v0 = &polygon.vertex[0 + OFFSET];
        let v1 = &polygon.vertex[1 + OFFSET];
        let v2 = &polygon.vertex[2 + OFFSET];

        let mut verts = [v0, v1, v2];

        let mut abc = Self::edge_function(&v0.0, &v1.0, &v2.0);
        if abc < 0 {
            verts.swap(0, 1);
            abc = -abc;
        }

        let (a, ac, a_uv) = verts[0];
        let (b, bc, b_uv) = verts[1];
        let (c, cc, c_uv) = verts[2];

        if abc == 0 {
            return 0;
        }

        let tl_ab = Self::is_top_left(a, b);
        let tl_bc = Self::is_top_left(b, c);
        let tl_ca = Self::is_top_left(c, a);

        let bias_ab = if tl_ab { 0 } else { -1 };
        let bias_bc = if tl_bc { 0 } else { -1 };
        let bias_ca = if tl_ca { 0 } else { -1 };

        let min_x = cmp::max(0, cmp::min(a.x, cmp::min(b.x, c.x)));
        let max_x = cmp::min(1024, cmp::max(a.x, cmp::max(b.x, c.x)));
        let min_y = cmp::max(0, cmp::min(a.y, cmp::min(b.y, c.y)));
        let max_y = cmp::min(512, cmp::max(a.y, cmp::max(b.y, c.y)));

        if max_x <= min_x || max_y <= min_y {
            return 0;
        }

        if max_x - min_x > 1023 || max_y - min_y > 512 {
            return 0;
        }

        let mut pixels = 0;

        let (texture_page_x, texture_page_y, texture_depth, clut_x, clut_y, texture_semi_transparency) = if let Some(t) = &polygon.texture {
            (t.page_base_x, t.page_base_y, t.texture_depth, t.clut_x, t.clut_y, t.semi_transparency)
        } else {
            (0, 0, TextureDepth::T4Bit, 0, 0, SemiTransparency::Average)
        };

        // Pre-calculate texture window values
        let tex_window_mask_x = (self.texture.window_x_mask as u32) << 3;
        let tex_window_mask_y = (self.texture.window_y_mask as u32) << 3;
        let tex_window_offset_x = ((self.texture.window_x_offset & self.texture.window_x_mask) as u32) << 3;
        let tex_window_offset_y = ((self.texture.window_y_offset & self.texture.window_y_mask) as u32) << 3;

        // Fixed point 16.16
        let inv_abc_fp = ((1u64 << 32) / abc as u64) as i64;

        let mut r_step_x = 0i64;
        let mut g_step_x = 0i64;
        let mut b_step_x = 0i64;
        let mut u_step_x = 0i64;
        let mut v_step_x = 0i64;

        if is_gouraud || is_textured {
            let dx_bc = b.y - c.y;
            let dx_ca = c.y - a.y;

            if is_gouraud {
                r_step_x = ((ac.r as i32 * dx_bc as i32 + bc.r as i32 * dx_ca as i32 + cc.r as i32 * (-(dx_bc + dx_ca) as i32)) as i64 * inv_abc_fp) >> 16;
                g_step_x = ((ac.g as i32 * dx_bc as i32 + bc.g as i32 * dx_ca as i32 + cc.g as i32 * (-(dx_bc + dx_ca) as i32)) as i64 * inv_abc_fp) >> 16;
                b_step_x = ((ac.b as i32 * dx_bc as i32 + bc.b as i32 * dx_ca as i32 + cc.b as i32 * (-(dx_bc + dx_ca) as i32)) as i64 * inv_abc_fp) >> 16;
            }

            if is_textured {
                u_step_x = (( (a_uv.u * 256 + 128) * dx_bc as i32 + (b_uv.u * 256 + 128) * dx_ca as i32 + (c_uv.u * 256 + 128) * (-(dx_bc + dx_ca) as i32)) as i64 * inv_abc_fp) >> 24;
                v_step_x = (( (a_uv.v * 256 + 128) * dx_bc as i32 + (b_uv.v * 256 + 128) * dx_ca as i32 + (c_uv.v * 256 + 128) * (-(dx_bc + dx_ca) as i32)) as i64 * inv_abc_fp) >> 24;
            }
        }

        for y in min_y..max_y {
            let p_y = Vertex { x: min_x, y };
            let mut abp = Self::edge_function(a, b, &p_y);
            let mut bcp = Self::edge_function(b, c, &p_y);
            let mut cap = Self::edge_function(c, a, &p_y);

            let dx_ab = a.y - b.y;
            let dx_bc = b.y - c.y;
            let dx_ca = c.y - a.y;

            let vram_y_offset = self.get_vram_offset_15(0, y as u16);
            let is_inside_y = y >= self.drawing_area.area_top as i16 && y <= self.drawing_area.area_bottom as i16;

            if is_inside_y {
                let w_a = (bcp as i64 * inv_abc_fp) >> 16;
                let w_b = (cap as i64 * inv_abc_fp) >> 16;
                let w_c = 65536i64 - w_a - w_b;

                let mut curr_r = ac.r as i64 * w_a + bc.r as i64 * w_b + cc.r as i64 * w_c;
                let mut curr_g = ac.g as i64 * w_a + bc.g as i64 * w_b + cc.g as i64 * w_c;
                let mut curr_b = ac.b as i64 * w_a + bc.b as i64 * w_b + cc.b as i64 * w_c;
                let mut curr_u = ((a_uv.u * 256 + 128) as i64 * w_a + (b_uv.u * 256 + 128) as i64 * w_b + (c_uv.u * 256 + 128) as i64 * w_c) >> 8;
                let mut curr_v = ((a_uv.v * 256 + 128) as i64 * w_a + (b_uv.v * 256 + 128) as i64 * w_b + (c_uv.v * 256 + 128) as i64 * w_c) >> 8;

                let area_left = self.drawing_area.area_left as i16;
                let area_right = self.drawing_area.area_right as i16;

                // Determine if we need to dither (gouraud shading or modulation)
                let dither_enabled = (is_gouraud || (is_textured && !is_raw_texture)) && self.dithering;

                for x in min_x..max_x {
                    let inside = (abp + bias_ab >= 0) && (bcp + bias_bc >= 0) && (cap + bias_ca >= 0);

                    if inside && x >= area_left && x <= area_right {
                        let mut color = if is_gouraud {
                            Color::new((curr_r >> 16) as u8, (curr_g >> 16) as u8, (curr_b >> 16) as u8, false)
                        } else {
                            *ac
                        };

                        let mut semi_transparency_mode = self.semi_transparency;
                        let mut transparent_pixel = false;
                        let mut texture_semi_transparency_allowed = false;

                        if is_textured {
                            semi_transparency_mode = texture_semi_transparency;

                            let u = ((curr_u >> 16) as u32 & !tex_window_mask_x) | tex_window_offset_x;
                            let v = ((curr_v >> 16) as u32 & !tex_window_mask_y) | tex_window_offset_y;

                            let y_tex = ((texture_page_y as u32) << 8) + v;
                            let texture_pixel = match texture_depth {
                                TextureDepth::T4Bit => {
                                    let vram_x_pixels = (((texture_page_x as u32) << 6) + (u >> 2)) & 0x3FF;
                                    let byte_offset = (((y_tex << 10) + vram_x_pixels) as usize) << 1;
                                    let value = self.vram[byte_offset] as u16 | ((self.vram[byte_offset + 1] as u16) << 8);
                                    let clut_index = ((value >> ((u & 3) << 2)) & 0xF) as usize;
                                    let clut_addr = ((((clut_y << 10) + (clut_x << 4)) as usize) << 1) + (clut_index << 1);
                                    self.vram[clut_addr] as u16 | ((self.vram[clut_addr + 1] as u16) << 8)
                                }
                                TextureDepth::T8Bit => {
                                    let vram_x_pixels = (((texture_page_x as u32) << 6) + (u >> 1)) & 0x3FF;
                                    let byte_offset = (((y_tex << 10) + vram_x_pixels) as usize) << 1;
                                    let value = self.vram[byte_offset] as u16 | ((self.vram[byte_offset + 1] as u16) << 8);
                                    let clut_index = ((value >> ((u & 1) << 3)) & 0xFF) as usize;
                                    let clut_addr = ((((clut_y << 10) + (clut_x << 4)) as usize) << 1) + (clut_index << 1);
                                    self.vram[clut_addr] as u16 | ((self.vram[clut_addr + 1] as u16) << 8)
                                }
                                _ => {
                                    let vram_x_pixels = (((texture_page_x as u32) << 6) + u) & 0x3FF;
                                    let byte_offset = (((y_tex << 10) + vram_x_pixels) as usize) << 1;
                                    self.vram[byte_offset] as u16 | ((self.vram[byte_offset + 1] as u16) << 8)
                                }
                            };

                            transparent_pixel = texture_pixel == 0x0000;
                            if !transparent_pixel {
                                texture_semi_transparency_allowed = (texture_pixel & 0x8000) != 0;
                                let raw_color = Color::from_u16(texture_pixel);
                                if is_raw_texture {
                                    color = raw_color;
                                } else {
                                    color = raw_color.modulate_with(&color);
                                }
                            }
                        }

                        if !transparent_pixel {
                            pixels += 1;
                            let is_semi_transparent_pixel = is_semi_transparent && (!is_textured || texture_semi_transparency_allowed);

                            let final_color = if dither_enabled {
                                let dither_value = DITHER_TABLE[(y & 3) as usize][(x & 3) as usize];
                                color.dither(dither_value)
                            } else {
                                color
                            };

                            let offset = vram_y_offset + ((x as usize & 0x3FF) << 1);
                            self.draw_pixel_offset(offset, final_color.to_u16(), true, is_semi_transparent_pixel, Some(semi_transparency_mode));
                        }
                    }
                    abp += dx_ab as i32;
                    bcp += dx_bc as i32;
                    cap += dx_ca as i32;
                    if is_gouraud {
                        curr_r += r_step_x;
                        curr_g += g_step_x;
                        curr_b += b_step_x;
                    }
                    if is_textured {
                        curr_u += u_step_x;
                        curr_v += v_step_x;
                    }
                }
            } else {
                for _ in min_x..max_x {
                    abp += dx_ab as i32;
                    bcp += dx_bc as i32;
                    cap += dx_ca as i32;
                }
            }
        }
        pixels
    }
}