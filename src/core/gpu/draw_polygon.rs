use tracing::{info, warn};
use crate::core::gpu::{Color, Gp0State, SemiTransparency, TextureDepth, Vertex, GPU};

#[derive(Debug,Clone,Default)]
struct PolygonTexture {
    clut_x: u32,
    clut_y: u32,
    page_base_x: u8,
    page_base_y: u8,
    semi_transparency: SemiTransparency,
    texture_depth: TextureDepth,
}

#[derive(Debug,Clone,Default)]
struct UV {
    u: u32,
    v: u32,
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
    pub(super) fn operation_polygon_rendering(&mut self,cmd:u32) {
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
            }
            Gp0State::WaitingCommandParameters(_, Some(_)) => {
                let cmd = self.cmd_fifo.pop().unwrap();
                let is_gouraud = (cmd & (1 << 28)) != 0;
                let is_textured = (cmd & (1 << 26)) != 0;
                let semi_transparent = (cmd & (1 << 25)) != 0;
                let raw_texture = (cmd & (1 << 24)) != 0;
                let shading_color = Color::from_u32(cmd);

                if is_gouraud && is_textured {
                    warn!("GPU: draw_polygon primitive with both gouraud shading and texture");
                }

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
                        uv.u = base_u as u32;
                        uv.v = base_v as u32;
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
                                    _ => unreachable!()
                                };
                                texture.page_base_x = page_base_x;
                                texture.page_base_y = page_base_y;
                                texture.texture_depth = texture_depth;
                                texture.semi_transparency = semi_transparency;
                            }
                            _ => {}
                        }
                    }
                    polygon.vertex.push((vertex,color,uv));
                    word_index += 1;
                }
                self.draw_polygon(&polygon,is_gouraud,is_textured,semi_transparent,raw_texture);
                self.gp0state = Gp0State::WaitingCommand;
            }
            _ => {}
        }
    }

    fn draw_polygon(&mut self, polygon:&Polygon,is_gouraud:bool,is_textured:bool,is_semi_transparent:bool,is_raw_texture:bool) {
        info!("Drawing polygon: {:?} gouraud={is_gouraud} textured={is_textured} semi_transparent={is_semi_transparent} raw_texture={is_raw_texture}",polygon);
        self.draw_triangle::<0>(polygon,is_gouraud,is_textured,is_semi_transparent,is_raw_texture);
        if polygon.vertex.len() == 4 {
            self.draw_triangle::<1>(polygon,is_gouraud,is_textured,is_semi_transparent,is_raw_texture);
        }
    }

    #[inline]
    fn lerp_u8(a: u8, b: u8, t: i32, t_max: i32) -> u8 {
        if t_max == 0 {
            return a;
        }
        let v = a as i32 + ((b as i32 - a as i32) * t) / t_max;
        v.clamp(0, 255) as u8
    }

    fn draw_triangle<const OFFSET : usize>(&mut self, polygon:&Polygon,is_gouraud:bool,is_textured:bool,is_semi_transparent:bool,is_raw_texture:bool) {
        let v0 = &polygon.vertex[0 + OFFSET];
        let v1 = &polygon.vertex[1 + OFFSET];
        let v2 = &polygon.vertex[2 + OFFSET];

        let mut verts = [v0,v1,v2];
        // sort by Y-coord
        verts.sort_by_key(|v| v.0.y);

        let (v0,c0,uv0) = verts[0];
        let (v1,c1,uv1) = verts[1];
        let (v2,c2,uv2) = verts[2];

        if v0.y == v2.y {
            return;
        }

        let mut draw_span = |y: i16, x0: i32, c0: Color, x1: i32, c1: Color| {
            let mut xs = x0 >> 16;
            let mut xe = x1 >> 16;

            let mut c_start = c0;
            let mut c_end   = c1;

            if xs > xe {
                std::mem::swap(&mut xs, &mut xe);
                std::mem::swap(&mut c_start, &mut c_end);
            }

            let dx = (xe - xs).max(1);
            /*
            The PS1 GPU uses what is called the top-left rule.
            If a pixel lies exactly on one of the triangle’s edges, only rasterize it if it’s on a top edge or a left edge.
            If it’s on a right edge or a bottom edge, skip it.
            This causes pixels lying on shared edges to only be rasterized once without needing to explicitly check if the edge is shared with another triangle.
             */
            for x in xs..xe {
                let i = x - xs;
                let color = if is_gouraud {
                    Color::new(Self::lerp_u8(c_start.r, c_end.r, i, dx),Self::lerp_u8(c_start.g, c_end.g, i, dx),Self::lerp_u8(c_start.b, c_end.b, i, dx),c_start.m | c_end.m)
                }
                else {
                    c0
                };
                // Dither enable (in Texpage command) affects ONLY polygons that do use gouraud shading or modulation.
                self.draw_pixel(&Vertex { x: xs as i16 + i as i16, y }, &color, is_semi_transparent, is_gouraud || (is_textured && !is_raw_texture));
            }

        };

        // upper half =============================================================================================
        let dy01 = (v1.y - v0.y) as i32;
        let dy02 = (v2.y - v0.y) as i32;

        for y in v0.y..v1.y {
            let t = (y - v0.y) as i32;
            let x01 = ((v0.x as i32) << 16) + (((v1.x as i32 - v0.x as i32) << 16) * t) / dy01.max(1);
            let x02 = ((v0.x as i32) << 16) + (((v2.x as i32 - v0.x as i32) << 16) * t) / dy02.max(1);

            let (c01,c02) = if is_gouraud {
                (Color {
                    r: Self::lerp_u8(c0.r, c1.r, t, dy01),
                    g: Self::lerp_u8(c0.g, c1.g, t, dy01),
                    b: Self::lerp_u8(c0.b, c1.b, t, dy01),
                    m: c0.m,
                },
                 Color {
                     r: Self::lerp_u8(c0.r, c2.r, t, dy02),
                     g: Self::lerp_u8(c0.g, c2.g, t, dy02),
                     b: Self::lerp_u8(c0.b, c2.b, t, dy02),
                     m: c0.m,
                 }
                )
            }
            else {
                (*c0,*c0)
            };

            draw_span(y, x01, c01, x02, c02);
        }
        // lower half =============================================================================================
        let dy12 = (v2.y - v1.y) as i32;

        for y in v1.y..v2.y {
            let t1 = (y - v1.y) as i32;
            let t2 = (y - v0.y) as i32;

            let x12 = ((v1.x as i32) << 16) + (((v2.x as i32 - v1.x as i32) << 16) * t1) / dy12.max(1);
            let x02 = ((v0.x as i32) << 16) + (((v2.x as i32 - v0.x as i32) << 16) * t2) / dy02.max(1);

            let c12 = if is_gouraud {
                Color {
                    r: Self::lerp_u8(c1.r, c2.r, t1, dy12),
                    g: Self::lerp_u8(c1.g, c2.g, t1, dy12),
                    b: Self::lerp_u8(c1.b, c2.b, t1, dy12),
                    m: c1.m,
                }
            }
            else {
                *c0
            };
            let c02 = if is_gouraud {
                Color {
                    r: Self::lerp_u8(c0.r, c2.r, t2, dy02),
                    g: Self::lerp_u8(c0.g, c2.g, t2, dy02),
                    b: Self::lerp_u8(c0.b, c2.b, t2, dy02),
                    m: c0.m,
                }
            }
            else {
                *c0
            };

            draw_span(y, x12, c12, x02, c02);
        }
    }
}