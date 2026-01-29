use tracing::{debug, info};
use crate::core::gpu::{Color, Gp0State, Vertex, GPU};

impl GPU {
    /*
    GPU Render Rectangle Commands
    Rectangles are drawn much faster than polygons. Unlike polygons, Gouraud shading is not possible, dithering isn't applied, the rectangle must forcefully have horizontal and vertical edges, textures cannot be rotated or scaled, and, of course, the GPU does render Rectangles as a single entity, without splitting them into two triangles. Note that this is sometimes refered to as a "sprite".

    The Rectangle command can be decoded using the following bitfield:

     bit number   value   meaning
      31-29        011    rectangle render
      28-27        sss    rectangle size
        26         1/0    textured / untextured
        25         1/0    semi-transparent / opaque
        24         1/0    raw texture / modulation
       23-0        rgb    first color value.
    The size parameter can be seen as the following enum:

      0 (00)      variable size
      1 (01)      single pixel (1x1)
      2 (10)      8x8 sprite
      3 (11)      16x16 sprite
    Therefore, the whole draw call can be seen as the following sequence of words:

    Color         ccBBGGRR    - command + color; color is ignored when textured
    Vertex1       YYYYXXXX    - required, indicates the upper left corner to render
    UV            ClutVVUU    - optional, only present for textured rectangles
    Width+Height  YsizXsiz    - optional, dimensions for variable sized rectangles (max 1023x511)
    Unlike for Textured-Polygons, the "Texpage" must be set up separately for Rectangles, via GP0(E1h). Width and Height can be up to 1023x511, however, the maximum size of the texture window is 256x256 (so the source data will be repeated when trying to use sizes larger than 256x256).

    If using a texture with a rectangle primitive, please that the texture UV, as well as the texture width must be even. If not, there will be one pixel sampling errors in the drawn rectangle every 16 pixels.

    Texture Origin and X/Y-Flip
    Vertex & Texcoord specify the upper-left edge of the rectangle. And, normally, screen coords and texture coords are both incremented during rendering the rectangle pixels.
    Optionally, X/Y-Flip bits can be set in Texpage.Bit12/13, these bits cause the texture coordinates to be decremented (instead of incremented). The X/Y-Flip bits do affect only Rectangles (not Polygons, nor VRAM Transfers).
    Caution: Reportedly, the X/Y-Flip feature isn't supported on old PSX consoles (unknown which ones exactly, maybe such with PU-7 mainboards, and unknown how to detect flipping support; except of course by reading VRAM).
     */
    pub(super) fn operation_rectangle_rendering(&mut self,cmd:u32) {
        match self.gp0state {
            Gp0State::WaitingCommandParameters(operation, None) => {
                let mut expected_data = 1usize;
                let size = (cmd >> 27) & 3;
                let is_textured = ((cmd >> 26) & 1) != 0;
                if is_textured {
                    expected_data += 1;
                }
                if size == 0b00 {
                    expected_data += 1;
                }

                self.gp0state = Gp0State::WaitingCommandParameters(operation, Some(expected_data));
            }
            Gp0State::WaitingCommandParameters(_, Some(_)) => {
                let cmd = self.cmd_fifo.pop().unwrap();
                let size = (cmd >> 27) & 3;
                let is_textured = ((cmd >> 26) & 1) != 0;
                let semi_transparent = ((cmd >> 25) & 1) != 0;
                let is_raw_texture = ((cmd >> 24) & 1) != 0;
                let shading_color = Color::from_u32(cmd);
                let mut vertex = Vertex::from_command_parameter(self.cmd_fifo.pop().unwrap());
                let uv = if is_textured { Some(self.cmd_fifo.pop().unwrap()) } else { None };
                let (width,height) = match size {
                    0b00 => {
                        let size = self.cmd_fifo.pop().unwrap();
                        ((size & 0x3FF) as u16,(size >> 16) as u16)
                    }
                    0b01 => (1,1),
                    0b10 => (8,8),
                    0b11 => (16,16),
                    _ => unreachable!()
                };
                debug!("GP0 Rectangle rendering (x,y)={:?} size={:02b} color={:?} textured_uv={:?} width={} height={} semi_transparent={} raw_texture={} x_offset={} y_offset={} texture={:?}",vertex,size,shading_color,uv,width,height,semi_transparent,is_raw_texture,self.drawing_area.x_offset,self.drawing_area.y_offset,self.texture);
                vertex.add_offset(self.drawing_area.x_offset,self.drawing_area.y_offset);
                let origin = vertex.clone();
                let color = shading_color.to_u16();
                match uv {
                    Some(uv) => { // textured
                        let base_u = uv as u8;
                        let base_v = (uv >> 8) as u8;
                        // if self.texture.rectangle_x_flip {
                        //     base_u = base_u.wrapping_add(width as u8);
                        // }
                        // if self.texture.rectangle_y_flip {
                        //     base_v = base_v.wrapping_add(height as u8);
                        // }
                        let clut = uv >> 16;
                        let clut_x = clut & 0x3F; // 0-5    X coordinate X/16
                        let clut_y = (clut >> 6) & 0x1FF; // 6-14   Y coordinate 0-511
                        for y in 0..height {
                            let v = base_v.wrapping_add(y as u8);
                            // let v = if self.texture.rectangle_y_flip {
                            //     base_v.wrapping_sub(y as u8)
                            // }
                            // else {
                            //     base_v.wrapping_add(y as u8)
                            // };
                            for x in 0..width {
                                if vertex.is_inside_drawing_area(&self.drawing_area) {
                                    let u = base_u.wrapping_add(x as u8);
                                    // let u = if self.texture.rectangle_x_flip {
                                    //     base_u.wrapping_sub(x as u8)
                                    // }
                                    // else {
                                    //     base_u.wrapping_add(x as u8)
                                    // };
                                    let texture_pixel = self.get_texture_pixel(clut_x, clut_y, u.into(), v.into(),self.texture.page_base_x,self.texture.page_base_y,self.texture.depth);

                                    if texture_pixel != 0x0000 {
                                        let raw_color = Color::from_u16(texture_pixel);
                                        let color = if is_raw_texture {
                                            raw_color
                                        } else {
                                            raw_color.modulate_with(&shading_color)
                                        };
                                        self.draw_pixel_offset(self.get_vram_offset_15(vertex.x as u16, vertex.y as u16), color.to_u16(), true, semi_transparent);
                                    }
                                }
                                vertex.x += 1;
                            }
                            vertex.x = origin.x;
                            vertex.y += 1;
                        }
                    }
                    None => { // non-textured
                        for y in 0..height {
                            for x in 0..width {
                                if vertex.is_inside_drawing_area(&self.drawing_area) {
                                    self.draw_pixel_offset(self.get_vram_offset_15(vertex.x as u16, vertex.y as u16), color, true, semi_transparent);
                                }
                                vertex.x += 1;
                            }
                            vertex.x = origin.x;
                            vertex.y += 1;
                        }
                    }
                }

                self.gp0state = Gp0State::WaitingCommand;
            }
            _ => {}
        }
    }
}