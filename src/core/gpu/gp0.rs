use super::{Color, GP0Operation, Gp0State, SemiTransparency, TextureDepth, VRamCopyConfig, Vertex, GPU};
use crate::core::clock::{Clock, EventType};
use crate::core::interrupt::{InterruptType, IrqHandler};
use tracing::{debug, error, info, warn};
use crate::core::gpu::timings::GPUTimings;

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
    /// returns (<if the operation needs parameters>,op)
    fn cmd_to_operation(cmd:u32) -> Option<(bool,GP0Operation)> {
        match (cmd >> 29) & 7 {
            0b001 => Some((true,GPU::operation_polygon_rendering)),
            0b010 => Some((true,GPU::operation_line_rendering)),
            0b011 => Some((true,GPU::operation_rectangle_rendering)),
            0b100 => Some((true,GPU::operation_vram_vram_copy)),
            0b101 => Some((true,GPU::operation_cpu_to_vram_copy)),
            0b110 => Some((true,GPU::operation_vram_to_cpu_copy)),
            _ => match cmd >> 24 {
                0x00 => Some((false,GPU::operation_nop)),
                0x01 => Some((false,GPU::operation_flush_texture_cache)),
                0x02 => Some((true,GPU::operation_quick_vram_fill)),
                0xE1 => Some((false,GPU::gp0_draw_mode_settings)),
                0xE2 => Some((false,GPU::gp0_texture_window_settings)),
                0xE3 => Some((false,GPU::gp0_set_drawing_area_top_left)),
                0xE4 => Some((false,GPU::gp0_set_drawing_area_bottom_right)),
                0xE5 => Some((false,GPU::gp0_set_drawing_offset)),
                0xE6 => Some((false,GPU::gp0_mask_bit_settings)),
                0x1F => Some((false,GPU::gp0_set_irq)),
                0x04..=0x1E | 0xE0 | 0xE7..=0xEF => {
                    debug!("Issue a GPU command mirroring 0x00: {cmd}");
                    Some((false,GPU::operation_nop))
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
    pub fn gp0_cmd(&mut self,cmd:u32,clock:&mut Clock,interrupt_handler:&mut IrqHandler) -> bool {
        if !self.ready_bits.ready_to_receive_cmd_word { // a command is executing
            if !self.gp0_fifo.push(cmd) {
                debug!("GP0 command queue is full!");
                return true; // notify externally that the queue is full, so the caller can stop sending commands until it's not full anymore
            }
            return false;
        }
        match self.gp0state {
            Gp0State::WaitingCommand => {
                debug!("GPU GP0 command {:08X}",cmd);
                match Self::cmd_to_operation(cmd) {
                    Some((needs_params,operation)) => {
                        if needs_params {
                            if !self.cmd_fifo.push(cmd) {
                                warn!("GP0 FIFO is full while pushing cmd {:08X}",cmd);
                            }
                            self.gp0state = Gp0State::WaitingCommandParameters(operation,None);
                        }

                        operation(self,cmd,interrupt_handler);
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
                else { // parameters completed, executing command
                    let cycles = operation(self, cmd,interrupt_handler);
                    self.schedule_command_completion(cycles,clock);
                }
            }
            Gp0State::WaitingPolyline(operation,arg_size,v,c,shaded,semi_transparency) => {
                debug!("GPU GP0 polyline parameter {:08X}",cmd);
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
                    else { // parameters completed, executing command
                        let cycles = operation(self, cmd,interrupt_handler);
                        self.schedule_command_completion(cycles,clock);
                    }
                }
            }
            Gp0State::VRamCopy(operation, config) => {
                debug!("GPU GP0\tdata {:08X}",cmd);
                operation(self, cmd,interrupt_handler);
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
        false
    }

    fn schedule_command_completion(&mut self,cycles:usize,clock:&mut Clock) {
        if cycles > 0 {
            self.ready_bits.ready_to_receive_cmd_word = false;
            //clock.schedule_gpu_dot_clock(EventType::GPUCommandCompleted, cycles as u64, self.display_config.h_res.get_divider());
            clock.schedule_gpu(EventType::GPUCommandCompleted, cycles as u64);
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
    pub(super) fn gp0_draw_mode_settings(&mut self,cmd:u32,_irq_handler:&mut IrqHandler) -> usize {
        self.texture.page_base_x = (cmd & 0xF) as u8;
        self.texture.page_base_y = ((cmd >> 4) & 0x1) as u8;
        self.semi_transparency = SemiTransparency::from_command(cmd);
        self.texture.depth = match (cmd >> 7) & 3 {
            0 => TextureDepth::T4Bit,
            1 => TextureDepth::T8Bit,
            2 => TextureDepth::T15Bit,
            3 => TextureDepth::Reserved,
            _ => unreachable!()
        };
        self.dithering = ((cmd >> 9) & 1) != 0;
        self.drawing_area.draw_to_display = ((cmd >> 10) & 1) != 0;
        //self.texture.disabled = ((cmd >> 11) & 1) != 0; // only for V2
        self.texture.rectangle_x_flip = ((cmd >> 12) & 1) != 0;
        self.texture.rectangle_y_flip = ((cmd >> 13) & 1) != 0;
        debug!("GP0(E1){:04X} Draw mode settings texture.page_base_x={:02X} texture.page_base_y={:02X} semi_transparency={:?} texture.depth={:?} dithering={} drawing_area.draw_to_display={} texture.rectangle_x_flip={} texture.rectangle_y_flip={}",
            cmd,
            self.texture.page_base_x,
            self.texture.page_base_y,
            self.semi_transparency,
            self.texture.depth,
            self.dithering,
            self.drawing_area.draw_to_display,
            //self.texture.disabled,
            self.texture.rectangle_x_flip,
            self.texture.rectangle_y_flip
        );
        0
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
    pub(super) fn gp0_texture_window_settings(&mut self,cmd:u32,_irq_handler:&mut IrqHandler) -> usize {
        self.texture.window_x_mask = (cmd & 0x1F) as u8;
        self.texture.window_y_mask = ((cmd >> 5) & 0x1F) as u8;
        self.texture.window_x_offset = ((cmd >> 10) & 0x1F) as u8;
        self.texture.window_y_offset = ((cmd >> 15) & 0x1F) as u8;
        debug!("GP0(E2) Texture window settings texture.window_x_mask={:05b} texture.window_y_mask={:05b} texture.window_x_offset={} texture.window_y_offset={}",self.texture.window_x_mask,self.texture.window_y_mask,self.texture.window_x_offset,self.texture.window_y_offset);
        0
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
    pub(super) fn gp0_set_drawing_area_top_left(&mut self,cmd:u32,_irq_handler:&mut IrqHandler) -> usize {
        self.drawing_area.area_left = (cmd & 0x3FF) as u16;
        self.drawing_area.area_top = ((cmd >> 10) & 0x1FF) as u16;
        debug!("GP0(E3) Set drawing area top left drawing_area.area_left={} drawing_area.area_top={}",self.drawing_area.area_left,self.drawing_area.area_top);
        0
    }
    pub(super) fn gp0_set_drawing_area_bottom_right(&mut self,cmd:u32,_irq_handler:&mut IrqHandler) -> usize {
        self.drawing_area.area_right = (cmd & 0x3FF) as u16;
        self.drawing_area.area_bottom = ((cmd >> 10) & 0x1FF) as u16;
        debug!("GP0(E4) Set drawing area bottom right drawing_area.area_right={} drawing_area.area_bottom={}",self.drawing_area.area_right,self.drawing_area.area_bottom);
        0
    }
    /*
    GP0(E5h) - Set Drawing Offset (X,Y)
      0-10   X-offset (-1024..+1023) (usually within X1,X2 of Drawing Area)
      11-21  Y-offset (-1024..+1023) (usually within Y1,Y2 of Drawing Area)
      22-23  Not used (zero)
      24-31  Command  (E5h)
    If you have configured the GTE to produce vertices with coordinate "0,0" being located in the center of the drawing area, then the Drawing Offset must be "X1+(X2-X1)/2, Y1+(Y2-Y1)/2". Or, if coordinate "0,0" shall be the upper-left of the Drawing Area, then Drawing Offset should be "X1,Y1". Where X1,Y1,X2,Y2 are the values defined with GP0(E3h-E4h).
     */
    pub(super) fn gp0_set_drawing_offset(&mut self,cmd:u32,_irq_handler:&mut IrqHandler) -> usize {
        self.drawing_area.x_offset = (((cmd & 0x7FF) << 5) as i16) >> 5;
        self.drawing_area.y_offset = ((((cmd >> 11) & 0x7FF) << 5) as i16) >> 5;
        debug!("GP0(E5) Set drawing offset drawing_area.x_offset={} drawing_area.y_offset={}",self.drawing_area.x_offset,self.drawing_area.y_offset);
        0
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
    pub(super) fn gp0_mask_bit_settings(&mut self,cmd:u32,_irq_handler:&mut IrqHandler) -> usize {
        self.force_set_mask_bit = (cmd & 1) != 0;
        self.preserve_masked_pixels = (cmd & 2) != 0;
        debug!("GP0(E6) Mask bit settings force_set_mask_bit={} preserve_masked_pixels={}",self.force_set_mask_bit,self.preserve_masked_pixels);
        0
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
    pub(super) fn draw_pixel_offset(&mut self, offset:usize, pixel:u16, use_mask:bool, semi_transparent:bool,semi_transparency: Option<SemiTransparency>) {
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
                pixel_to_write = semi_transparency.unwrap().blend_rgb555(pixel_to_write,old_pixel);
            }
        }
        else if semi_transparent {
            let old_pixel = self.get_pixel_15(offset);
            pixel_to_write = semi_transparency.unwrap().blend_rgb555(pixel_to_write,old_pixel);
        }
        
        self.vram[offset] = pixel_to_write as u8;
        self.vram[offset + 1] = (pixel_to_write >> 8) as u8;
    }

    #[inline(always)]
    pub(super) fn draw_pixel(&mut self,v:&Vertex,color:&Color,semi_transparent:bool,semi_transparency: Option<SemiTransparency>,allow_dithering:bool) {
        if /*self.drawing_area.draw_to_display &&*/ v.is_inside_drawing_area(&self.drawing_area) {
            let color = if allow_dithering && self.dithering {
                let dither_value = DITHER_TABLE[(v.y & 3) as usize][(v.x & 3) as usize];
                color.dither(dither_value)
            }
            else {
                *color
            };
            self.draw_pixel_offset(self.get_vram_offset_15(v.x as u16, v.y as u16), color.to_u16(), true, semi_transparent,semi_transparency);
        }
    }



    pub(super) fn get_texture_pixel(&self, clut_x: u32, clut_y: u32, u: u32, v: u32, texture_page_x:u8, texture_page_y: u8, texture_depth:TextureDepth) -> u16 {
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

        let y = ((texture_page_y as u32) << 8) + v;

        match texture_depth {
            TextureDepth::T4Bit => {
                let vram_x_pixels = (((texture_page_x as u32) << 6) + (u >> 2)) & 0x3FF;
                let byte_offset = (((y << 10) + vram_x_pixels) as usize) << 1;
                let value = self.vram[byte_offset] as u16 | ((self.vram[byte_offset + 1] as u16) << 8);
                let shift = (u & 3) << 2;
                let clut_index = ((value >> shift) & 0xF) as u32;
                let clut_base_addr = (((clut_y << 10) + (clut_x << 4)) as usize) << 1;
                let clut_addr = clut_base_addr + ((clut_index as usize) << 1);
                self.vram[clut_addr] as u16 | ((self.vram[clut_addr + 1] as u16) << 8)
            }
            TextureDepth::T8Bit => {
                let vram_x_pixels = (((texture_page_x as u32) << 6) + (u >> 1)) & 0x3FF;
                let byte_offset = (((y << 10) + vram_x_pixels) as usize) << 1;
                let value = self.vram[byte_offset] as u16 | ((self.vram[byte_offset + 1] as u16) << 8);
                let shift = (u & 1) << 3;
                let clut_index = ((value >> shift) & 0xFF) as u32;
                let clut_base_addr = (((clut_y << 10) + (clut_x << 4)) as usize) << 1;
                let clut_addr = clut_base_addr + ((clut_index as usize) << 1);

                self.vram[clut_addr] as u16 | ((self.vram[clut_addr + 1] as u16) << 8)
            }
            TextureDepth::T15Bit | TextureDepth::Reserved => {
                let vram_x_pixels = (((texture_page_x as u32) << 6) + u) & 0x3FF;
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
    fn operation_vram_vram_copy(&mut self,_cmd:u32,_irq_handler:&mut IrqHandler) -> usize {
        match self.gp0state {
            Gp0State::WaitingCommandParameters(operation, None) => {
                self.gp0state = Gp0State::WaitingCommandParameters(operation, Some(3));
                0
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
                        self.draw_pixel_offset(dest_offset, pixel, true,false,None);
                    }
                }
                self.gp0state = Gp0State::WaitingCommand;
                GPUTimings::vram_to_vram_copy(x_size as usize,y_size as usize)
            },
            _ => {
                0
            }
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
    fn operation_cpu_to_vram_copy(&mut self, word:u32,_irq_handler:&mut IrqHandler) -> usize {
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
                self.draw_pixel_offset(self.get_vram_offset_15(vram_x + 1, vram_y), (word >> 16) as u16, true, false,None);
                debug!("Cpu->VRam ({vram_x},{vram_y}) = {:04X}",word >> 16);
                self.draw_pixel_offset(self.get_vram_offset_15(vram_x, vram_y), word as u16, true, false,None);
                debug!("Cpu->VRam ({},{vram_y}) = {:04X}",vram_x + 1,word as u16);
            }
            _ => {}
        }
        0
    }
    fn operation_vram_to_cpu_copy(&mut self,_cmd:u32,_irq_handler:&mut IrqHandler) -> usize {
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
        0
    }
    fn operation_nop(&mut self,_cmd:u32,_irq_handler:&mut IrqHandler) -> usize {
        self.cmd_fifo.pop(); // discard command
        0
    }
    fn operation_flush_texture_cache(&mut self,cmd:u32,_irq_handler:&mut IrqHandler) -> usize {
        // TODO: implement texture cache flushing
        self.cmd_fifo.pop(); // discard command
        debug!("GPU GP0 Flush Texture Cache command {:08X} - not implemented",cmd);
        0
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
    fn operation_quick_vram_fill(&mut self,_cmd:u32,_irq_handler:&mut IrqHandler) -> usize {
        match self.gp0state {
            Gp0State::WaitingCommandParameters(operation, None) => {
                self.gp0state = Gp0State::WaitingCommandParameters(operation, Some(2));
                0
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
                        self.draw_pixel_offset(self.get_vram_offset_15(x_pos + x, y_pos + y), fill_color, false, false,None);
                    }
                }
                self.gp0state = Gp0State::WaitingCommand;
                GPUTimings::rectangle_fill(width,height)
            },
            _ => {
                0
            }
        }
    }

    /*
    GP0(1Fh) - Interrupt Request (IRQ1)
      1st  Command           (Cc000000h)                    ;GPUSTAT.24
    Requests IRQ1. Can be acknowledged via GP1(02h). This feature is rarely used.
    Note: The command is used by Blaze'n'Blade, but the game doesn't have IRQ1 enabled, and the written value (1F801810h) looks more like an I/O address, rather than like a command, so not sure if it's done intentionally, or if it is just a bug.
     */
    fn gp0_set_irq(&mut self,_cmd:u32,irq_handler:&mut IrqHandler) -> usize {
        self.irq = true;
        irq_handler.set_irq(InterruptType::GPU);
        debug!("GP0 IRQ Request");
        0
    }
}