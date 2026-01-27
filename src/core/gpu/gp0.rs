use tracing::{debug, error, info, warn};
use super::{Color, GP0Operation, Gp0State, SemiTransparency, TextureDepth, VRamCopyConfig, Vertex, GPU};

const DITHER_TABLE: &[[i8; 4]; 4] = &[[-4, 0, -3, 1], [2, -2, 3, -1], [-3, 1, -4, 0], [3, -1, 2, -2]];

/*
• GPU GP0 COMMAND SUMMARY
• The GP0 port (0x1F801810) handles rendering and VRAM transfer commands.
• Commands are 32-bit words, where the top 3 bits usually identify the type.
• Extra parameters are sent as subsequent 32-bit words.
• | Code (Hex/Bits) | Command Name           | Parameters | Notes                                            |
• |-----------------|------------------------|------------|--------------------------------------------------|
• | 0x20-3F (001b)  | Polygon Rendering      | 3 - 12     | Variable based on Vertices (3/4), Shading, Texture
• | 0x40-5F (010b)  | Line Rendering         | 2+         | Polyline uses variable words until 0x50005000 term
• | 0x60-7F (011b)  | Rectangle Rendering    | 1 - 3      | Variable based on Size (fixed/var) and Texture
• | 0x80-9F (100b)  | VRAM-to-VRAM Copy      | 3          | Source Coord, Dest Coord, and Width+Height
• | 0xA0-BF (101b)  | CPU-to-VRAM Copy       | 2+         | Dest Coord, Width+Height, then Pixel Data words
• | 0xC0-DF (110b)  | VRAM-to-CPU Copy       | 2          | Source Coord and Width+Height
• | 0x00            | No-op (NOP)            | 0          | Often used as filler in command lists
• | 0x01            | Flush Texture Cache    | 0          | Ensures VRAM consistency after modifications
• | 0x02            | Quick VRAM Fill        | 2          | Solid 15-bit color fill; ignores masking settings
• | 0xE1            | Draw Mode Setting      | 0          | Immediate: Sets TPage, Dithering, and Texture Mode
• | 0xE2            | Texture Window         | 0          | Immediate: Sets Mask and Offset for texture tiling
• | 0xE3            | Draw Area Top Left     | 0          | Immediate: Sets (X1, Y1) for clipping
• | 0xE4            | Draw Area Bottom Right | 0          | Immediate: Sets (X2, Y2) for clipping
• | 0xE5            | Draw Offset            | 0          | Immediate: (X,Y) added to all drawing coords
• | 0xE6            | Mask Bit Setting       | 0          | Immediate: Controls pixel protection/forcing
• NOTE: Environment commands (0xE1-0xE6) are immediate and do not take up space
• in the 64-byte command FIFO.
  GP0(04h..1Eh,E0h,E7h..EFh) - Mirrors of GP0(00h) - NOP (?)
  Like GP0(00h), these commands don't take up space in the FIFO. So, maybe, they are same as GP0(00h),
  however, the Drawing Area/Offset commands GP0(E3h..E5h) don't take up FIFO space either, so not taking up FIFO space doesn't necessarily mean that the command has no function.
*/
impl GPU {
    /// returns (<if the operation needs parameters>,<if the operation uses FIFO>,op)
    fn cmd_to_operation(cmd:u32) -> Option<(bool,bool,GP0Operation)> {
        match (cmd >> 29) & 7 {
            0b001 => Some((true,true,GPU::operation_polygon_rendering)),
            0b010 => Some((true,true,GPU::operation_line_rendering)),
            0b011 => Some((true,true,GPU::operation_rectangle_rendering)),
            0b100 => Some((true,true,GPU::operation_vram_vram_copy)),
            0b101 => Some((true,true,GPU::operation_cpu_to_vram_copy)),
            0b110 => Some((true,true,GPU::operation_vram_to_cpu_copy)),
            _ => match cmd >> 24 {
                0x00 => Some((false,false,GPU::operation_nop)),
                0x01 => Some((false,true,GPU::operation_flush_texture_cache)),
                0x02 => Some((true,true,GPU::operation_quick_vram_fill)),
                0xE1 => Some((false,true,GPU::gp0_draw_mode_settings)),
                0xE2 => Some((false,true,GPU::gp0_texture_window_settings)),
                0xE3 => Some((false,false,GPU::gp0_set_drawing_area_top_left)),
                0xE4 => Some((false,false,GPU::gp0_set_drawing_area_bottom_right)),
                0xE5 => Some((false,false,GPU::gp0_set_drawing_offset)),
                0xE6 => Some((false,false,GPU::gp0_mask_bit_settings)),
                0x1F => Some((false,true,GPU::gp0_set_irq)),
                0x04..=0x1E | 0xE0 | 0xE7..=0xEF => {
                    debug!("Issue a GPU command mirroring 0x00: {cmd}");
                    Some((false,false,GPU::operation_nop))
                }
                _ => {
                    None
                }
            }
        }
    }
    /*
    GP0(E3h..E5h) do not take up space in the FIFO, so they are probably executed immediately (even if there're still other commands in the FIFO).
    Best use them only if you are sure that the FIFO is empty (otherwise the new Drawing Area settings might accidentally affect older Rendering Commands in the FIFO).
     */
    pub fn gp0_cmd(&mut self,cmd:u32) {
        match self.gp0state {
            Gp0State::WaitingCommand => {
                debug!("GPU GP0 command {:08X}",cmd);
                match Self::cmd_to_operation(cmd) {
                    Some((needs_params,use_fifo,operation)) => {
                        if use_fifo {
                            if !self.cmd_fifo.push(cmd) {
                                warn!("GP0 FIFO is full while pushing cmd {:08X}",cmd);
                            }
                        }
                        if needs_params {
                            self.gp0state = Gp0State::WaitingCommandParameters(operation,None);
                        }
                        else {
                            self.cmd_fifo.pop(); // remove command from FIFO
                        }
                        operation(self,cmd);
                    }
                    None => {
                        warn!("GPU GP0 unknown command {:08X}",cmd);
                    }
                }
            }
            Gp0State::WaitingCommandParameters(operation, Some(pars)) => {
                debug!("GPU GP0\tparameter {:08X}",cmd);
                if !self.cmd_fifo.push(cmd) {
                    warn!("GP0 FIFO is full while pushing parameter {:08X}",cmd);
                }
                if pars > 1 {
                    self.gp0state = Gp0State::WaitingCommandParameters(operation,Some(pars - 1));
                }
                else {
                    operation(self, cmd);
                }
            }
            Gp0State::WaitingPolyline(operation,arg_size,v,c,shaded,semi_transparency) => {
                if (cmd & 0xF000F000) == 0x50005000 { // polyline terminated
                    self.gp0state = Gp0State::WaitingCommand;
                }
                else {
                    if !self.cmd_fifo.push(cmd) {
                        warn!("GP0 FIFO is full while pushing parameter {:08X}",cmd);
                    }
                    if arg_size > 1 {
                        self.gp0state = Gp0State::WaitingPolyline(operation,arg_size - 1,v,c,shaded,semi_transparency);
                    }
                    else {
                        operation(self, cmd);
                    }
                }
            }
            Gp0State::VRamCopy(operation, config) => {
                debug!("GPU GP0\tdata {:08X}",cmd);
                operation(self, cmd);
                match config.next() {
                    Some(next) => {
                        self.gp0state = Gp0State::VRamCopy(operation,next)
                    }
                    None => {
                        debug!("Cpu->VRam operation terminated.");
                        self.gp0state = Gp0State::WaitingCommand;
                    }
                }
            }
            _ => unreachable!(),
        }
    }
    
    /*
    GP0(E1h) - Draw Mode setting (aka "Texpage")
      0-3   Texture page X Base   (N*64) (ie. in 64-halfword steps)    ;GPUSTAT.0-3
      4     Texture page Y Base 1 (N*256) (ie. 0, 256, 512 or 768)     ;GPUSTAT.4
      5-6   Semi-transparency     (0=B/2+F/2, 1=B+F, 2=B-F, 3=B+F/4)   ;GPUSTAT.5-6
      7-8   Texture page colors   (0=4bit, 1=8bit, 2=15bit, 3=Reserved);GPUSTAT.7-8
      9     Dither 24bit to 15bit (0=Off/strip LSBs, 1=Dither Enabled) ;GPUSTAT.9
      10    Drawing to display area (0=Prohibited, 1=Allowed)          ;GPUSTAT.10
      11    Texture page Y Base 2 (N*512) (only for 2 MB VRAM)         ;GPUSTAT.15
      12    Textured Rectangle X-Flip   (BIOS does set this bit on power-up...?)
      13    Textured Rectangle Y-Flip   (BIOS does set it equal to GPUSTAT.13...?)
      14-23 Not used (should be 0)
      24-31 Command  (E1h)
    The GP0(E1h) command is required only for Lines, Rectangle, and Untextured-Polygons (for Textured-Polygons, the data is specified in form of the Texpage attribute; except that, Bits 9-10 can be changed only via GP0(E1h), not via the Texpage attribute).
    Texture page colors setting 3 (reserved) is same as setting 2 (15bit).
    Bits 4 and 11 are the LSB and MSB of the 2-bit texture page Y coordinate. Normally only bit 4 is used as retail consoles only have 1 MB VRAM. Setting bit 11 (Y>=512) on a retail console with a v2 GPU will result in textures disappearing if 2 MB VRAM support was previously enabled using GP1(09h), as the VRAM chip select will no longer be active. Bit 11 is always ignored by v0 GPUs that do not support 2 MB VRAM.
    Note: GP0(00h) seems to be often inserted between Texpage and Rectangle commands, maybe it acts as a NOP, which may be required between that commands, for timing reasons...?
     */
    pub(super) fn gp0_draw_mode_settings(&mut self,cmd:u32) {
        self.texture.page_base_x = (cmd & 0xF) as u8;
        self.texture.page_base_y = ((cmd >> 4) & 0x1) as u8;
        self.semi_transparency = SemiTransparency::from_command(cmd);
        self.texture.depth = match (cmd >> 7) & 3 {
            0 => TextureDepth::T4Bit,
            1 => TextureDepth::T8Bit,
            2 => TextureDepth::T15Bit,
            3 => {
                warn!("GPU gp0 draw mode settings with texture page colors with value 3 (reserved). Default to 2");
                TextureDepth::T15Bit
            }
            _ => unreachable!()
        };
        self.dithering = ((cmd >> 9) & 1) != 0;
        self.drawing_area.draw_to_display = ((cmd >> 10) & 1) != 0;
        self.texture.disabled = ((cmd >> 11) & 1) != 0; // only for V2
        self.texture.rectangle_x_flip = ((cmd >> 12) & 1) != 0;
        self.texture.rectangle_y_flip = ((cmd >> 13) & 1) != 0;
        debug!("GP0(E1) Draw mode settings texture.page_base_x={:02X} texture.page_base_y={:02X} semi_transparency={:?} texture.depth={:?} dithering={} drawing_area.draw_to_display={} texture.disabled={} texture.rectangle_x_flip={} texture.rectangle_y_flip={}",
            self.texture.page_base_x,
            self.texture.page_base_y,
            self.semi_transparency,
            self.texture.depth,
            self.dithering,
            self.drawing_area.draw_to_display,
            self.texture.disabled,
            self.texture.rectangle_x_flip,
            self.texture.rectangle_y_flip
        );
    }
    /*
    GP0(E2h) - Texture Window setting
      0-4    Texture window Mask X   (in 8 pixel steps)
      5-9    Texture window Mask Y   (in 8 pixel steps)
      10-14  Texture window Offset X (in 8 pixel steps)
      15-19  Texture window Offset Y (in 8 pixel steps)
      20-23  Not used (zero)
      24-31  Command  (E2h)
    Mask specifies the bits that are to be manipulated, and Offset contains the new values for these bits, ie. texture X/Y coordinates are adjusted as so:
      Texcoord = (Texcoord AND (NOT (Mask * 8))) OR ((Offset AND Mask) * 8)
    The area within a texture window is repeated throughout the texture page. The data is not actually stored all over the texture page but the GPU reads the repeated patterns as if they were there. Considering all possible regular tilings of UV coordinates for powers of two, the texture window primitive can be constructed as follows using a desired set of parameters of tiling_x, tiling_y, window_pos_x, window_pos_y, u, v and color_mode:
    x_tiling_factor = {8: 0b11111, 16: 0b11110, 32: 0b11100, 64: 0b11000, 128: 0b10000, 256: 0b00000}[tiling_x]
    y_tiling_factor = {8: 0b11111, 16: 0b11110, 32: 0b11100, 64: 0b11000, 128: 0b10000, 256: 0b00000}[tiling_y]
    x_offset = u & 0b11111
    x_offset <<= {15: 0, 8: 1, 4: 2}[color_mode]
    x_offset >>= 3;
    y_offset = v & 0b11111
    y_offset >>= 3
    texture_window_prim = (0xE20 << 20) | (y_offset << 15) | (x_offset << 10) | (y_tiling_factor << 5) | x_tiling_factor
     */
    pub(super) fn gp0_texture_window_settings(&mut self,cmd:u32) {
        self.texture.window_x_mask = (cmd & 0x1F) as u8;
        self.texture.window_y_mask = ((cmd >> 5) & 0x1F) as u8;
        self.texture.window_x_offset = ((cmd >> 10) & 0x1F) as u8;
        self.texture.window_y_offset = ((cmd >> 15) & 0x1F) as u8;
        debug!("GP0(E2) Texture window settings texture.window_x_mask={:05b} texture.window_y_mask={:05b} texture.window_x_offset={} texture.window_y_offset={}",self.texture.window_x_mask,self.texture.window_y_mask,self.texture.window_x_offset,self.texture.window_y_offset);
    }
    /*
    GP0(E3h) - Set Drawing Area top left (X1,Y1)
    GP0(E4h) - Set Drawing Area bottom right (X2,Y2)
      0-9    X-coordinate (0..1023)
      10-18  Y-coordinate (0..511)   ;\on v0 GPU (max 1 MB VRAM)
      19-23  Not used (zero)         ;/
      10-19  Y-coordinate (0..1023)  ;\on v2 GPU (max 2 MB VRAM)
      20-23  Not used (zero)         ;/
      24-31  Command  (Exh)
    Sets the drawing area corners. The Render commands GP0(20h..7Fh) are automatically clipping any pixels that are outside of this region.
     */
    pub(super) fn gp0_set_drawing_area_top_left(&mut self,cmd:u32) {
        self.drawing_area.area_left = (cmd & 0x3FF) as u16;
        self.drawing_area.area_top = ((cmd >> 10) & 0x1FF) as u16;
        debug!("GP0(E3) Set drawing area top left drawing_area.area_left={} drawing_area.area_top={}",self.drawing_area.area_left,self.drawing_area.area_top);
    }
    pub(super) fn gp0_set_drawing_area_bottom_right(&mut self,cmd:u32) {
        self.drawing_area.area_right = (cmd & 0x3FF) as u16;
        self.drawing_area.area_bottom = ((cmd >> 10) & 0x1FF) as u16;
        debug!("GP0(E4) Set drawing area bottom right drawing_area.area_right={} drawing_area.area_bottom={}",self.drawing_area.area_right,self.drawing_area.area_bottom);
    }
    /*
    GP0(E5h) - Set Drawing Offset (X,Y)
      0-10   X-offset (-1024..+1023) (usually within X1,X2 of Drawing Area)
      11-21  Y-offset (-1024..+1023) (usually within Y1,Y2 of Drawing Area)
      22-23  Not used (zero)
      24-31  Command  (E5h)
    If you have configured the GTE to produce vertices with coordinate "0,0" being located in the center of the drawing area, then the Drawing Offset must be "X1+(X2-X1)/2, Y1+(Y2-Y1)/2". Or, if coordinate "0,0" shall be the upper-left of the Drawing Area, then Drawing Offset should be "X1,Y1". Where X1,Y1,X2,Y2 are the values defined with GP0(E3h-E4h).
     */
    pub(super) fn gp0_set_drawing_offset(&mut self,cmd:u32) {
        self.drawing_area.x_offset = (((cmd & 0x7FF) << 5) as i16) >> 5;
        self.drawing_area.y_offset = ((((cmd >> 11) & 0x7FF) << 5) as i16) >> 5;
        debug!("GP0(E5) Set drawing offset drawing_area.x_offset={} drawing_area.y_offset={}",self.drawing_area.x_offset,self.drawing_area.y_offset);
    }
    /*
    GP0(E6h) - Mask Bit Setting
      0     Set mask while drawing (0=TextureBit15, 1=ForceBit15=1)   ;GPUSTAT.11
      1     Check mask before draw (0=Draw Always, 1=Draw if Bit15=0) ;GPUSTAT.12
      2-23  Not used (zero)
      24-31 Command  (E6h)
    When bit0 is off, the upper bit of the data written to the framebuffer is equal to bit15 of the texture color (ie. it is set for colors that are marked as "semi-transparent") (for untextured polygons, bit15 is set to zero).
    When bit1 is on, any (old) pixels in the framebuffer with bit15=1 are write-protected, and cannot be overwritten by (new) rendering commands.
    The mask setting affects all rendering commands, as well as CPU-to-VRAM and VRAM-to-VRAM transfer commands (where it acts on the separate halfwords, ie. as for 15bit textures). However, Mask does NOT affect the Fill-VRAM command.
    This setting is used in games such as Metal Gear Solid and Silent Hill.
     */
    pub(super) fn gp0_mask_bit_settings(&mut self,cmd:u32) {
        self.force_set_mask_bit = (cmd & 1) != 0;
        self.preserve_masked_pixels = (cmd & 2) != 0;
        debug!("GP0(E6) Mask bit settings force_set_mask_bit={} preserve_masked_pixels={}",self.force_set_mask_bit,self.preserve_masked_pixels);
    }

    /*
    Wrapping
    If the Source/Dest starting points plus the width/height value exceed the 1024x512 pixel VRAM size, then the Copy/Fill operations wrap to the opposite memory edge (without any carry-out from X to Y, nor from Y to X).
     */
    #[inline]
    pub(super) fn get_vram_offset_15(&self, x:u16, y:u16) -> usize {
        (((y & 0x1FF) as usize) << 11) + (((x & 0x3FF) as usize) << 1) // y * 2048 + x * 2
    }
    #[inline]
    pub(super) fn get_vram_offset_24(&self, x:u16, y:u16) -> usize {
        (((y & 0x1FF) as usize) << 11) + (((x & 0x3FF) as usize) * 3)
    }
    #[inline]
    pub(super) fn get_pixel_15(&self, offset:usize) -> u16 {
        self.vram[offset] as u16 | (self.vram[offset + 1] as u16) << 8
    }
    #[inline]
    fn draw_pixel_offset(&mut self, offset:usize, pixel:u16, use_mask:bool, semi_transparent:bool) {
        let mut pixel_to_write = pixel;
        if use_mask {
            let old_pixel = self.get_pixel_15(offset);
            if self.preserve_masked_pixels {
                if (old_pixel & 0x8000) != 0 { // pixel is protected
                    return;
                }
            }
            if self.force_set_mask_bit {
                pixel_to_write |= 0x8000;
            }
            if semi_transparent {
                pixel_to_write = self.semi_transparency.blend_rgb555(pixel_to_write,old_pixel);
            }
        }
        else if semi_transparent {
            let old_pixel = self.get_pixel_15(offset);
            pixel_to_write = self.semi_transparency.blend_rgb555(pixel_to_write,old_pixel);
        }
        
        self.vram[offset] = pixel_to_write as u8;
        self.vram[offset + 1] = (pixel_to_write >> 8) as u8;
    }
    // Operations ====================================================================
    fn operation_polygon_rendering(&mut self,cmd:u32) {
        todo!("GPU polygon rendering command {:08X} not implemented yet",cmd);
    }
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
    fn operation_line_rendering(&mut self,cmd:u32) {
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
    fn draw_line(&mut self, start: &Vertex, end: &Vertex, start_color: &Color, end_color: &Color,shaded:bool, semi_transparent: bool) {
        let dx = start.dx(end).abs() as i32;
        let dy = start.dy(end).abs() as i32;

        // The GPU will not render any lines or polygons where the distance between any two vertices is
        // larger than 1023 horizontally or 511 vertically
        if dx > 1023 || dy > 511 {
            return;
        }

        let total_steps = dx.max(dy);

        if total_steps == 0 {
            self.draw_pixel(start,start_color,semi_transparent);
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

            self.draw_pixel(&v, &Color::new(r,g,b,false), semi_transparent);

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
    #[inline(always)]
    fn draw_pixel(&mut self,v:&Vertex,color:&Color,semi_transparent:bool) {
        if v.is_inside_drawing_area(&self.drawing_area) {
            let color = if self.dithering {
                let dither_value = DITHER_TABLE[(v.y & 3) as usize][(v.x & 3) as usize];
                color.dither(dither_value)
            }
            else {
                *color
            };
            self.draw_pixel_offset(self.get_vram_offset_15(v.x as u16, v.y as u16), color.to_u16(), true, semi_transparent);
        }
    }

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
    fn operation_rectangle_rendering(&mut self,cmd:u32) {
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
                                    let texture_pixel = self.get_texture_pixel(clut_x, clut_y, u.into(), v.into());

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

    fn get_texture_pixel(&self,
                         clut_x: u32,
                         clut_y: u32,
                         u: u32,
                         v: u32) -> u16 {
        /*
        GP0(E2h) - Texture Window setting
          0-4    Texture window Mask X   (in 8 pixel steps)
          5-9    Texture window Mask Y   (in 8 pixel steps)
          10-14  Texture window Offset X (in 8 pixel steps)
          15-19  Texture window Offset Y (in 8 pixel steps)
          20-23  Not used (zero)
          24-31  Command  (E2h)
        Mask specifies the bits that are to be manipulated, and Offset contains the new values for these bits, ie. texture X/Y coordinates are adjusted as so:
          Texcoord = (Texcoord AND (NOT (Mask * 8))) OR ((Offset AND Mask) * 8)
         */
        let u = (u & !((self.texture.window_x_mask as u32) << 3)) | (((self.texture.window_x_offset & self.texture.window_x_mask) as u32) << 3);
        let v = (v & !((self.texture.window_y_mask as u32) << 3)) | (((self.texture.window_y_offset & self.texture.window_y_mask) as u32) << 3);

        let y = ((self.texture.page_base_y as u32) << 8) + v;

        match self.texture.depth {
            TextureDepth::T4Bit => {
                let vram_x_pixels = (((self.texture.page_base_x as u32) << 6) + (u >> 2)) & 0x3FF;
                let byte_offset = (((y << 10) + vram_x_pixels) as usize) << 1;
                let value = self.vram[byte_offset] as u16 | ((self.vram[byte_offset + 1] as u16) << 8);
                let shift = (u & 3) << 2;
                let clut_index = ((value >> shift) & 0xF) as u32;
                let clut_base_addr = (((clut_y << 10) + (clut_x << 4)) as usize) << 1;
                let clut_addr = clut_base_addr + ((clut_index as usize) << 1);
                self.vram[clut_addr] as u16 | ((self.vram[clut_addr + 1] as u16) << 8)
            }
            TextureDepth::T8Bit => {
                let vram_x_pixels = (((self.texture.page_base_x as u32) << 6) + (u >> 1)) & 0x3FF;
                let byte_offset = (((y << 10) + vram_x_pixels) as usize) << 1;
                let value = self.vram[byte_offset] as u16 | ((self.vram[byte_offset + 1] as u16) << 8);
                let shift = (u & 1) << 3;
                let clut_index = ((value >> shift) & 0xFF) as u32;
                let clut_base_addr = (((clut_y << 10) + (clut_x << 4)) as usize) << 1;
                let clut_addr = clut_base_addr + ((clut_index as usize) << 1);

                self.vram[clut_addr] as u16 | ((self.vram[clut_addr + 1] as u16) << 8)
            }
            TextureDepth::T15Bit => {
                let vram_x_pixels = (((self.texture.page_base_x as u32) << 6) + u) & 0x3FF;
                let byte_offset = (((y << 10) + vram_x_pixels) as usize) << 1;
                self.vram[byte_offset] as u16 | ((self.vram[byte_offset + 1] as u16) << 8)
            }
        }
    }

    /*
    VRAM to VRAM blitting - command 4 (100)
      1st  Command
      2nd  Source Coord      (YyyyXxxxh)  ;Xpos counted in halfwords
      3rd  Destination Coord (YyyyXxxxh)  ;Xpos counted in halfwords
      4th  Width+Height      (YsizXsizh)  ;Xsiz counted in halfwords
    Copies data within framebuffer. The transfer is affected by Mask setting.
     */
    fn operation_vram_vram_copy(&mut self,_cmd:u32) {
        match self.gp0state {
            Gp0State::WaitingCommandParameters(operation, None) => {
                self.gp0state = Gp0State::WaitingCommandParameters(operation, Some(3));
            }
            Gp0State::WaitingCommandParameters(_, Some(_)) => {
                // extract parameters
                self.cmd_fifo.pop(); // discard command
                let src_coord = self.cmd_fifo.pop().unwrap();
                let dest_coord = self.cmd_fifo.pop().unwrap();
                let width_height = self.cmd_fifo.pop().unwrap();
                let src_x = (src_coord as u16) & 0x3FF;
                let src_y = ((src_coord >> 16) as u16) & 0x1FF;
                let dest_x = (dest_coord as u16) & 0x3FF;
                let dest_y = ((dest_coord >> 16) as u16) & 0x1FF;
                let mut x_size = (((width_height as u16) - 1) & 0x3FF) + 1;
                if x_size == 0 {
                    x_size = 0x400
                }
                //x_size >>= 1; // x_size is in halfwords
                let mut y_size = ((((width_height >> 16) as u16) - 1) & 0x1FF) + 1;
                if y_size == 0 {
                    y_size = 0x200;
                }
                debug!("Executing Vram->Vram copy src=({},{}) dest=({},{}) x_size={} y_size={}",src_x,src_y,dest_x,dest_y,x_size,y_size);
                // perform copy
                for y in 0..y_size {
                    for x in 0..x_size {
                        let src_offset = self.get_vram_offset_15(src_x + x, src_y + y);
                        let pixel = self.get_pixel_15(src_offset);
                        let dest_offset = self.get_vram_offset_15(dest_x + x, dest_y + y);
                        self.draw_pixel_offset(dest_offset, pixel, true,false);
                    }
                }
                self.gp0state = Gp0State::WaitingCommand;
            },
            _ => {}
        }
    }
    /*
    Masking for COPY Commands parameters
      Xpos=(Xpos AND 3FFh)                       ;range 0..3FFh
      Ypos=(Ypos AND 1FFh)                       ;range 0..1FFh
      Xsiz=((Xsiz-1) AND 3FFh)+1                 ;range 1..400h
      Ysiz=((Ysiz-1) AND 1FFh)+1                 ;range 1..200h
    Parameters are just clipped to 10bit/9bit range, the only special case is that Size=0 is handled as Size=max.
     */
    fn extract_cpu_vram_copy_parameters<const CPU_TO_VRAM:bool>(&mut self, operation: GP0Operation) -> Gp0State {
        // discard command
        self.cmd_fifo.pop();
        let dest_coord = self.cmd_fifo.pop().unwrap();
        let width_height = self.cmd_fifo.pop().unwrap();
        let x_pos = (dest_coord as u16) & 0x3FF;
        let y_pos = ((dest_coord >> 16) as u16) & 0x1FF;
        let mut x_size = width_height as u16;
        if x_size == 0 {
            x_size = 0x400
        }
        else {
            x_size = ((x_size - 1) & 0x3FF) + 1;
        }
        let mut y_size = (width_height >> 16) as u16;
        if y_size == 0 {
            y_size = 0x200;
        }
        else {
            y_size = ((y_size - 1) & 0x1FF) + 1;
        }
        if CPU_TO_VRAM {
            debug!("Starting Cpu->VRam copy x_pos={x_pos} y_pos={y_pos} x_size={x_size} y_size={y_size}");
        }
        else {
            debug!("Starting VRam->Cpu copy x_pos={x_pos} y_pos={y_pos} x_size={x_size} y_size={y_size}");
        }
        Gp0State::VRamCopy(operation,VRamCopyConfig {
            coord_x: x_pos,
            coord_y: y_pos,
            counter_x: 0,
            counter_y: 0,
            width: x_size,
            height: y_size,
        })
    }
    /*
    CPU to VRAM blitting - command 5 (101)
      1st  Command
      2nd  Destination Coord (YyyyXxxxh)  ;Xpos counted in halfwords
      3rd  Width+Height      (YsizXsizh)  ;Xsiz counted in halfwords
      ...  Data              (...)      <--- usually transferred via DMA
     Transfers data from CPU to frame buffer. If the number of halfwords to be sent is odd, an extra halfword should be sent, as packets consist of 32bits words.
     The transfer is affected by Mask setting.
     */
    fn operation_cpu_to_vram_copy(&mut self, word:u32) {
        match self.gp0state {
            Gp0State::WaitingCommandParameters(operation,None) => {
                self.gp0state = Gp0State::WaitingCommandParameters(operation,Some(2));
            }
            Gp0State::WaitingCommandParameters(operation,Some(_)) => {
                self.gp0state = self.extract_cpu_vram_copy_parameters::<true>(operation);
            },
            Gp0State::VRamCopy(_,config) => {
                let vram_x = (config.coord_x.wrapping_add(config.counter_x)) & 0x3FF;
                let vram_y = (config.coord_y.wrapping_add(config.counter_y)) & 0x1FF;
                self.draw_pixel_offset(self.get_vram_offset_15(vram_x + 1, vram_y), (word >> 16) as u16, true, false);
                debug!("Cpu->VRam ({vram_x},{vram_y}) = {:04X}",word >> 16);
                self.draw_pixel_offset(self.get_vram_offset_15(vram_x, vram_y), word as u16, true, false);
                debug!("Cpu->VRam ({},{vram_y}) = {:04X}",vram_x + 1,word as u16);
            }
            _ => {}
        }
    }
    fn operation_vram_to_cpu_copy(&mut self,_cmd:u32) {
        match self.gp0state {
            Gp0State::WaitingCommandParameters(operation,None) => {
                self.gp0state = Gp0State::WaitingCommandParameters(operation,Some(2));
            }
            Gp0State::WaitingCommandParameters(operation,Some(_)) => {
                self.gp0state = self.extract_cpu_vram_copy_parameters::<false>(operation);
                self.ready_bits.ready_to_send_vram_to_cpu = true;
            }
            _ => {}
        }
    }
    fn operation_nop(&mut self,_cmd:u32) {
        self.cmd_fifo.pop(); // discard command
    }
    fn operation_flush_texture_cache(&mut self,cmd:u32) {

    }
    /*
    Quick Rectangle Fill
      1st  Color+Command     (02BbGgRrh)  ;24bit RGB value (see note)
      2nd  Top Left Corner   (YyyyXxxxh)  ;Xpos counted in halfwords, steps of 10h
      3rd  Width+Height      (YsizXsizh)  ;Xsiz counted in halfwords, steps of 10h
    Fills the area in the frame buffer with the value in RGB. Horizontally the filling is done in 16-pixel (32-bytes) units (see below masking/rounding).
    The "Color" parameter is a 24bit RGB value, however, the actual fill data is 16bit: The hardware linearly converts the 24bit RGB value to 15bit RGB by dropping the lower 3 bits of each color value and additionally sets the mask bit (bit15) to 0.
    Rectangle filling is not affected by the GP0(E6h) mask setting, acting as if GP0(E6h).0 and GP0(E6h).1 are both zero.
    This command is typically used to do a quick clear, as it'll be faster to run than an equivalent Render Rectangle command.
     */
    fn operation_quick_vram_fill(&mut self,_cmd:u32) {
        match self.gp0state {
            Gp0State::WaitingCommandParameters(operation, None) => {
                self.gp0state = Gp0State::WaitingCommandParameters(operation, Some(2));
            }
            Gp0State::WaitingCommandParameters(_operation, Some(_)) => {
                let fill_color = Color::from_u32(self.cmd_fifo.pop().unwrap()).to_u16();
                let top_left = self.cmd_fifo.pop().unwrap();
                let width_height = self.cmd_fifo.pop().unwrap();
                let x_pos = (top_left as u16) & 0x3F0;
                let y_pos = ((top_left >> 16) as u16) & 0x1FF;
                let width = (((width_height as u16) & 0x3FF) + 0xF) & !0xF;
                let height = ((width_height >> 16) as u16) & 0x1FF;
                debug!("Executing Quick VRam Fill color={:04X} pos=({},{}) width={} height={}",fill_color,x_pos,y_pos,width,height);
                for y in 0..height {
                    for x in 0..width {
                        self.draw_pixel_offset(self.get_vram_offset_15(x_pos + x, y_pos + y), fill_color, false, false);
                    }
                }
                self.gp0state = Gp0State::WaitingCommand;
            },
            _ => {}
        }
    }

    /*
    GP0(1Fh) - Interrupt Request (IRQ1)
      1st  Command           (Cc000000h)                    ;GPUSTAT.24
    Requests IRQ1. Can be acknowledged via GP1(02h). This feature is rarely used.
    Note: The command is used by Blaze'n'Blade, but the game doesn't have IRQ1 enabled, and the written value (1F801810h) looks more like an I/O address, rather than like a command, so not sure if it's done intentionally, or if it is just a bug.
     */
    fn gp0_set_irq(&mut self,_cmd:u32) {
        self.irq = true;
        debug!("GP0 IRQ Request");
    }
}