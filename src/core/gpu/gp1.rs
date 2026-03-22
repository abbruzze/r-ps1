use tracing::{debug, info, warn};
use crate::core::interrupt::IrqHandler;
use super::{DMADirection, DisplayDepth, Gp0State, VideoHorizontalResolution, VideoMode, VideoVerticalResolution, GPU};

impl GPU {
    /*
    GP1 Display Control Commands are sent by writing the 8bit Command number (MSBs), and 24bit parameter (LSBs) to Port 1F801814h.
    Unlike GP0 commands, GP1 commands are passed directly to the GPU (ie. they can be sent even when the FIFO is full).
     */
    pub fn gp1_cmd(&mut self,cmd:u32) {
        debug!("GPU GP1 command {:08X}",cmd);
        self.gp1_commands[(cmd >> 24) as usize](self,cmd & 0xFFFFFF);
    }

    // GP1 commands ==========================================
    pub(super) fn init_gp1_commands(&mut self) {
        for cmd in 0..0x100 { // GP1(40h..FFh) Mirrors of GP1(00h..3Fh).
            let function = match cmd & 0x3F {
                0x00 => GPU::gp1_reset_cpu,
                0x01 => GPU::gp1_reset_command_buffer,
                0x02 => GPU::gp1_ack_interrupt,
                0x03 => GPU::gp1_display_enable,
                0x04 => GPU::gp1_dma_direction,
                0x05 => GPU::gp1_start_of_display_area,
                0x06 => GPU::gp1_horizontal_display_range,
                0x07 => GPU::gp1_vertical_display_range,
                0x08 => GPU::gp1_display_mode,
                0x10..=0x1F => GPU::gp1_read_gpu_internal_register,
                _ => GPU::gp1_not_implemented,
            };
            self.gp1_commands[cmd as usize] = function;
        }
    }

    /*
    GP1(00h) - Reset GPU
      0-23  Not used (zero)
    Resets the GPU to the following values:
      GP1(01h)      ;clear fifo
      GP1(02h)      ;ack irq (0)
      GP1(03h)      ;display off (1)
      GP1(04h)      ;dma off (0)
      GP1(05h)      ;display address (0)
      GP1(06h)      ;display x1,x2 (x1=200h, x2=200h+256*10)
      GP1(07h)      ;display y1,y2 (y1=010h, y2=010h+240)
      GP1(08h)      ;display mode 320x200 NTSC (0)
      GP0(E1h..E6h) ;rendering attributes (0)
    Accordingly, GPUSTAT becomes 14802000h. The x1,y1 values are too small, ie. the upper-left edge isn't visible. Note that GP1(09h) is NOT affected by the reset command.
     */
    fn gp1_reset_cpu(&mut self,_cmd:u32) {
        debug!("GP1(00) Reset GPU");
        self.gp1_reset_command_buffer(0);
        self.gp1_ack_interrupt(0);
        self.gp1_display_enable(1);
        self.gp1_dma_direction(0);
        self.gp1_start_of_display_area(0);
        self.gp1_horizontal_display_range(0x200 | (0x200+256*10) << 12);
        self.gp1_vertical_display_range(0x10 | (0x10+240) << 10);
        self.gp1_display_mode(0);
        let mut fake_interrupt_handler = IrqHandler::new();
        self.gp0_draw_mode_settings(0,&mut fake_interrupt_handler);
        self.gp0_texture_window_settings(0,&mut fake_interrupt_handler);
        self.gp0_set_drawing_area_top_left(0,&mut fake_interrupt_handler);
        self.gp0_set_drawing_area_bottom_right(0,&mut fake_interrupt_handler);
        self.gp0_set_drawing_offset(0,&mut fake_interrupt_handler);
        self.gp0_mask_bit_settings(0,&mut fake_interrupt_handler);
        self.gp0state = Gp0State::WaitingCommand
    }
    /*
    GP1(01h) - Reset Command Buffer
      0-23  Not used (zero)
    Resets the command buffer and CLUT cache.
     */
    fn gp1_reset_command_buffer(&mut self,_cmd:u32) {
        debug!("GP1(01) Reset command buffer and CLUT cache");
        self.cmd_fifo.clear();
        self.gp0_fifo.clear();
        self.ready_bits.ready_to_receive_cmd_word = true;
        // TODO clear CLUT cache
    }
    /*
    GP1(02h) - Acknowledge GPU Interrupt (IRQ1)
      0-23  Not used (zero)                                        ;GPUSTAT.24
    Resets the IRQ flag in GPUSTAT.24. The flag can be set via GP0(1Fh).
     */
    fn gp1_ack_interrupt(&mut self,_cmd:u32) {
        debug!("GP1(02) Ack IRQ");
        self.irq = false;
    }
    /*
    GP1(03h) - Display Enable
      0     Display On/Off   (0=On, 1=Off)                         ;GPUSTAT.23
      1-23  Not used (zero)
    Turns display on/off. "Note that a turned off screen still gives the flicker of NTSC on a PAL screen if NTSC mode is selected."
    The "Off" settings displays a black picture (and still sends /SYNC signals to the television set). (Unknown if it still generates vblank IRQs though?)
     */
    fn gp1_display_enable(&mut self,cmd:u32) {
        self.display_config.display_disabled = cmd == 1;
        debug!("GP1(03) Display enabled {}",!self.display_config.display_disabled);
    }
    /*
    GP1(04h) - DMA Direction / Data Request
      0-1  DMA Direction (0=Off, 1=FIFO, 2=CPUtoGP0, 3=GPUREADtoCPU) ;GPUSTAT.29-30
      2-23 Not used (zero)
    Notes: Manually sending/reading data by software (non-DMA) is ALWAYS possible, regardless of the GP1(04h) setting. The GP1(04h) setting does affect the meaning of GPUSTAT.25.
     */
    fn gp1_dma_direction(&mut self,cmd:u32) {
        //self.ready_bits.ready_to_receive_dma_block = true;
        self.dma_direction = match cmd & 3 {
            0 => {
                //self.ready_bits.ready_to_receive_dma_block = false;
                DMADirection::Off
            },
            1 => DMADirection::Fifo,
            2 => DMADirection::CpuToGp0,
            3 => DMADirection::VRamToCpu,
            _ => unreachable!()
        };
        debug!("GP1(04) DMA direction {:06X}={:?}",cmd,self.dma_direction);
    }
    /*
    GP1(05h) - Start of Display area (in VRAM)
      0-9   X (0-1023)    (halfword address in VRAM)  (relative to begin of VRAM)
      10-18 Y (0-511)     (scanline number in VRAM)   (relative to begin of VRAM)
      19-23 Not used (zero)
    Upper/left Display source address in VRAM. The size and target position on screen is set via Display Range registers; target=X1,Y2; size=(X2-X1/cycles_per_pix), (Y2-Y1).
    Unknown if using Y values in 512-1023 range is supported (with 2 MB VRAM).
     */
    fn gp1_start_of_display_area(&mut self,cmd:u32) {
        self.display_config.vram_x_start = (cmd & 0x3FF) as u16;
        self.display_config.vram_y_start = ((cmd >> 10) & 0x1FF) as u16;
        debug!("GP1(05) Start of display AREA X={} Y={}",self.display_config.vram_x_start,self.display_config.vram_y_start);
    }
    /*
    GP1(06h) - Horizontal Display range (on Screen)
      0-11   X1 (260h+0)       ;12bit       ;\counted in video clock units,
      12-23  X2 (260h+320*8)   ;12bit       ;/relative to HSYNC
    Specifies the horizontal range within which the display area is displayed.
    For resolutions other than 320 pixels it may be necessary to fine adjust the value to obtain an exact match (eg. X2=X1+pixels*cycles_per_pix).
    The number of displayed pixels per line is "(((X2-X1)/cycles_per_pix)+2) AND NOT 3" (ie. the hardware is rounding the width up/down to a multiple of 4 pixels).
     */
    fn gp1_horizontal_display_range(&mut self,cmd:u32) {
        self.display_config.horizontal_start = (cmd & 0xFFF) as u16;
        self.display_config.horizontal_end = ((cmd >> 12) & 0xFFF) as u16;
        debug!("GP1(06) Horizontal display range X1={} X2={} X-SIZE={}",self.display_config.horizontal_start,self.display_config.horizontal_end,(self.display_config.horizontal_end - self.display_config.horizontal_start) as f64 / self.display_config.h_res.get_divider() as f64);
    }
    /*
    GP1(07h) - Vertical Display range (on Screen)
      0-9   Y1 (NTSC=88h-(240/2), (PAL=A3h-(288/2))  ;\scanline numbers on screen,
      10-19 Y2 (NTSC=88h+(240/2), (PAL=A3h+(288/2))  ;/relative to VSYNC
      20-23 Not used (zero)
    Specifies the vertical range within which the display area is displayed.
    The number of lines is Y2-Y1 (unlike as for the width, there's no rounding applied to the height).
    If Y2 is set to a much too large value, then the hardware stops to generate vblank interrupts (IRQ0).
     */
    fn gp1_vertical_display_range(&mut self,cmd:u32) {
        self.display_config.vertical_start = (cmd & 0x3FF) as u16;
        self.display_config.vertical_end = ((cmd >> 10) & 0x3FF) as u16;
        debug!("GP1(07) Vertical display range Y1={} Y2={} Y-SIZE={}",self.display_config.vertical_start,self.display_config.vertical_end,self.display_config.vertical_end - self.display_config.vertical_start);
    }
    /*
    GP1(08h) - Display mode
      0-1   Horizontal Resolution 1     (0=256, 1=320, 2=512, 3=640) ;GPUSTAT.17-18
      2     Vertical Resolution         (0=240, 1=480, when Bit5=1)  ;GPUSTAT.19
      3     Video Mode                  (0=NTSC/60Hz, 1=PAL/50Hz)    ;GPUSTAT.20
      4     Display Area Color Depth    (0=15bit, 1=24bit)           ;GPUSTAT.21
      5     Vertical Interlace          (0=Off, 1=On)                ;GPUSTAT.22
      6     Horizontal Resolution 2     (0=256/320/512/640, 1=368)   ;GPUSTAT.16
      7     Flip screen horizontally    (0=Off, 1=On, v1 only)       ;GPUSTAT.14
      8-23  Not used (zero)
    Note: Interlace must be enabled to see all lines in 480-lines mode (interlace causes ugly flickering, so a non-interlaced low resolution image typically has better quality than a high resolution interlaced image, a pretty bad example is the intro screens shown by the BIOS).
    The Display Area Color Depth bit does NOT affect GP0 draw commands, which always draw in 15 bit. However, the Vertical Interlace flag DOES affect GP0 draw commands.
    Bit 7 is known as "reverseflag" and can reportedly be used on (v1?) arcade/prototype GPUs to flip the screen horizontally.
    On a v2 GPU setting this bit corrupts the display output, possibly due to leftovers of the v1 GPU's screen flipping circuitry still being present.
     */
    fn gp1_display_mode(&mut self,cmd:u32) {
        self.display_config.h_res = VideoHorizontalResolution::from_gp1_08(cmd);
        self.display_config.v_res = if (cmd & 0x4) != 0 { VideoVerticalResolution::Y480Lines } else { VideoVerticalResolution::Y240Lines };
        self.display_config.video_mode = if (cmd & 0x8) != 0 { VideoMode::Pal } else { VideoMode::Ntsc };
        self.display_config.display_depth = if (cmd & 0x10) != 0 { DisplayDepth::D24Bits } else { DisplayDepth::D15Bits };
        self.display_config.interlaced = (cmd & 0x20) != 0;
        self.raster.total_lines = self.display_config.video_mode.total_lines();
        self.raster.total_cycles = self.display_config.video_mode.horizontal_cycles();
        if (cmd & 0x80) != 0 {
            warn!("GP1(08) Display mode with H flip - not supported on v2 GPU");
        }
        debug!("GP1(08) Display mode HRES={:?} VRES={:?} MODE={:?} DEPTH={:?} INTERLACED={}",self.display_config.h_res,self.display_config.v_res,self.display_config.video_mode,self.display_config.display_depth,self.display_config.interlaced);
    }
    /*
    GP1(10h) - Read GPU internal register
    GP1(11h..1Fh) - Mirrors of GP1(10h), Read GPU internal register
    After sending the command, the result can be read (immediately) from GPUREAD register (there's no NOP or other delay required)
    (namely GPUSTAT.Bit27 is used only for VRAM reads, but NOT for register reads, so do not try to wait for that flag).
    On v0 GPUs, the following indices are supported:
      00h-01h = Returns Nothing (old value in GPUREAD remains unchanged)
      02h     = Read Texture Window setting  ;GP0(E2h) ;20bit/MSBs=Nothing
      03h     = Read Draw area top left      ;GP0(E3h) ;19bit/MSBs=Nothing
      04h     = Read Draw area bottom right  ;GP0(E4h) ;19bit/MSBs=Nothing
      05h     = Read Draw offset             ;GP0(E5h) ;22bit
      06h-07h = Returns Nothing (old value in GPUREAD remains unchanged)
      08h-FFFFFFh = Mirrors of 00h..07h
    The selected data is latched in GPUREAD, the same/latched value can be read multiple times, but, the latch isn't automatically updated when changing GP0 registers.
     */
    fn gp1_read_gpu_internal_register(&mut self,cmd:u32) {
        debug!("GP1(10) Read GPU Internal Register {:08X}",cmd);
        let index = (cmd & 0x7) as u8;
        self.gpu_read_register = match index {
            0x00 | 0x01 | 0x06 | 0x07 => {
                // Returns Nothing (old value in GPUREAD remains unchanged)
                self.gpu_read_register
            }
            0x02 => {
                // GP0(E2h) - Texture Window setting
                //       0-4    Texture window Mask X   (in 8 pixel steps)
                //       5-9    Texture window Mask Y   (in 8 pixel steps)
                //       10-14  Texture window Offset X (in 8 pixel steps)
                //       15-19  Texture window Offset Y (in 8 pixel steps)
                self.texture.window_x_mask as u32 | (self.texture.window_y_mask as u32) << 5 | (self.texture.window_x_offset as u32) << 10 | (self.texture.window_y_offset as u32) << 15
            }
            0x03 => {
                // GP0(E3h) - Set Drawing Area top left (X1,Y1)
                //       0-9    X-coordinate (0..1023)
                //       10-18  Y-coordinate (0..511)   ;\on v0 GPU (max 1 MB VRAM)
                self.drawing_area.area_left as u32 | (self.drawing_area.area_top as u32) << 10
            }
            0x04 => {
                // GP0(E4h) - Set Drawing Area bottom right (X2,Y2)
                //       0-9    X-coordinate (0..1023)
                //       10-18  Y-coordinate (0..511)   ;\on v0 GPU (max 1 MB VRAM)
                self.drawing_area.area_right as u32 | (self.drawing_area.area_bottom as u32) << 10
            }
            0x05 => {
                // GP0(E5h) - Set Drawing Offset (X,Y)
                //       0-10   X-offset (-1024..+1023) (usually within X1,X2 of Drawing Area)
                //       11-21  Y-offset (-1024..+1023) (usually within Y1,Y2 of Drawing Area)
                self.drawing_area.x_offset as u32 | (self.drawing_area.y_offset as u32) << 10
            }
            _ => unreachable!()
        }
    }

    pub(super) fn gp1_not_implemented(&mut self,_cmd:u32) {
        warn!("GP1 command not implemented!!");
    }
}