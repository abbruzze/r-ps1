use crate::core::gpu::{Color, Gp0State, Vertex, GPU};
use crate::core::interrupt::IrqHandler;

impl GPU {
    /*
    GPU Render Line Commands
    When the upper 3 bits of the first GP0 command are set to 2 (010), then the command can be decoded using the following bitfield:

     bit number   value   meaning
      31-29        010    line render
        28         1/0    gouraud / flat shading
        27         1/0    polyline / single line
        25         1/0    semi-transparent / opaque
       23-0        rgb    first color value.
    So each vertex can be seen as the following list of words:

    Color      xxBBGGRR    - optional, only present for gouraud shading
    Vertex     YYYYXXXX    - required, two signed 16 bits values
    When polyline mode is active, at least two vertices must be sent to the GPU. The vertex list is terminated by the bits 12-15 and 28-31 equaling 0x5, or (word & 0xF000F000) == 0x50005000. The terminator value occurs on the first word of the vertex (i.e. the color word if it's a gouraud shaded).

    If the 2 vertices in a line overlap, then the GPU will draw a 1x1 rectangle in the location of the 2 vertices using the colour of the first vertex.

    Note
    Lines are displayed up to \<including> their lower-right coordinates (ie. unlike as for polygons, the lower-right coordinate is not excluded).
    If dithering is enabled (via Texpage command), then both monochrome and shaded lines are drawn with dithering (this differs from monochrome polygons and monochrome rectangles).

    Wire-Frame
    Poly-Lines can be used (among others) to create Wire-Frame polygons (by setting the last Vertex equal to Vertex 1).
     */
    pub(super) fn operation_line_rendering(&mut self,cmd:u32,_irq_handler:&mut IrqHandler) {
        match self.gp0state {
            Gp0State::WaitingCommandParameters(operation, None) => {
                let is_gouraud = (cmd & (1 << 28)) != 0;
                self.gp0state = Gp0State::WaitingCommandParameters(operation, Some(if is_gouraud {3} else {2}));
            }
            Gp0State::WaitingCommandParameters(operation, Some(_)) => {
                let cmd = self.cmd_fifo.pop().unwrap();
                let is_polyline = (cmd & (1 << 27)) != 0;
                let is_gouraud = (cmd & (1 << 28)) != 0;
                let semi_transparent = ((cmd >> 25) & 1) != 0;
                let start_color = Color::from_u32(cmd);

                let mut start_vertex = Vertex::from_command_parameter(self.cmd_fifo.pop().unwrap());
                start_vertex.add_offset(self.drawing_area.x_offset,self.drawing_area.y_offset);
                let end_color = if is_gouraud {
                    Color::from_u32(self.cmd_fifo.pop().unwrap())
                }
                else {
                    start_color
                };

                let orig_end_vertex = Vertex::from_command_parameter(self.cmd_fifo.pop().unwrap());
                let mut end_vertex = orig_end_vertex.clone();
                end_vertex.add_offset(self.drawing_area.x_offset,self.drawing_area.y_offset);
                //info!("Drawing line v1={:?}/{:?} v2={:?}{:?} shaded={is_gouraud} semi_transparent={semi_transparent}",start_vertex,start_color,end_vertex,end_color);
                self.draw_line(&start_vertex,&end_vertex,&start_color,&end_color,is_gouraud,semi_transparent);

                if is_polyline {
                    let arg_size : usize = if is_gouraud {2} else {1};
                    self.gp0state = Gp0State::WaitingPolyline(operation,arg_size,orig_end_vertex,end_color,is_gouraud,semi_transparent);
                }
                else {
                    self.gp0state = Gp0State::WaitingCommand;
                }
            }
            Gp0State::WaitingPolyline(operation,_,start_vertex,start_color,is_gouraud,semi_transparent) => {
                let end_color = if is_gouraud {
                    Color::from_u32(self.cmd_fifo.pop().unwrap())
                }
                else {
                    start_color
                };
                let orig_end_vertex = Vertex::from_command_parameter(self.cmd_fifo.pop().unwrap());
                let mut end_vertex = orig_end_vertex.clone();
                end_vertex.add_offset(self.drawing_area.x_offset,self.drawing_area.y_offset);
                //info!("Drawing polyline v1={:?}/{:?} v2={:?}{:?} shaded={is_gouraud} semi_transparent={semi_transparent}",start_vertex,start_color,end_vertex,end_color);
                self.draw_line(&start_vertex,&end_vertex,&start_color,&end_color,is_gouraud,semi_transparent);
                let arg_size : usize = if is_gouraud {2} else {1};
                self.gp0state = Gp0State::WaitingPolyline(operation,arg_size,orig_end_vertex,end_color,is_gouraud,semi_transparent);
            }
            _ => {}
        }
    }
    // Bresenham's line algorithm
    pub(super) fn draw_line(&mut self, start: &Vertex, end: &Vertex, start_color: &Color, end_color: &Color,shaded:bool, semi_transparent: bool) {
        let dx = start.dx(end).abs() as i32;
        let dy = start.dy(end).abs() as i32;

        // The GPU will not render any lines or polygons where the distance between any two vertices is
        // larger than 1023 horizontally or 511 vertically
        if dx > 1023 || dy > 511 {
            return;
        }

        let total_steps = dx.max(dy);

        if total_steps == 0 {
            self.draw_pixel(start,start_color,semi_transparent,Some(self.semi_transparency),true);
            return;
        }

        let sx = if start.x < end.x { 1 } else { -1 };
        let sy = if start.y < end.y { 1 } else { -1 };

        let mut err = dx - dy;
        let mut v = start.clone();

        let r_step = if shaded { ((end_color.r as i32 - start_color.r as i32) << 16) / total_steps } else { 0 };
        let g_step = if shaded { ((end_color.g as i32 - start_color.g as i32) << 16) / total_steps } else { 0 };
        let b_step = if shaded { ((end_color.b as i32 - start_color.b as i32) << 16) / total_steps } else { 0 };

        // Current rgb value in fixed-point (16.16)
        let mut r_current = (start_color.r as i32) << 16;
        let mut g_current = (start_color.g as i32) << 16;
        let mut b_current = (start_color.b as i32) << 16;

        loop {
            let r = (r_current >> 16) as u8;
            let g = (g_current >> 16) as u8;
            let b = (b_current >> 16) as u8;

            self.draw_pixel(&v, &Color::new(r,g,b,false), semi_transparent,Some(self.semi_transparency),true);

            if v.x == end.x && v.y == end.y {
                break;
            }

            let e2 = err << 1;

            if e2 > -dy {
                err -= dy;
                v.x += sx;
            }

            if e2 < dx {
                err += dx;
                v.y += sy;
            }

            if shaded {
                r_current += r_step;
                g_current += g_step;
                b_current += b_step;
            }
        }
    }
}