mod gp1;
mod gp0;
mod draw_line;
mod draw_rectangle;
mod draw_polygon;

use std::cmp;
use crate::core::clock::Clock;
use crate::core::clock::EventType;
use crate::core::dma::DmaDevice;
use crate::core::interrupt::{InterruptType, IrqHandler};
use crate::core::memory::bus::Bus;
use crate::renderer::{GPUFrameBuffer, Renderer};
use std::sync::Arc;
use tracing::{debug, info};
/*
GPU Versions
Summary of GPU Differences
  Differences...                v0 (160-pin)            v1 (208-pin prototype)  v2 (208-pin)
  GPU Chip                      CXD8514Q                CXD8538Q                CXD8561Q/BQ/CQ/CXD9500Q
  Mainboard                     EARLY-PU-8 and below    Arcade boards only      LATE-PU-8 and up
  Memory Type                   Dual-ported VRAM        Dual-ported VRAM?       Normal DRAM
  GPUSTAT.13 when interlace=off always 0                unknown                 always 1
  GPUSTAT.14                    always 0                screen flip             nonfunctional screen flip
  GPUSTAT.15                    always 0                always 0?               bit1 of texpage Y base
  GP1(10h:index3..4)            19-bit (1 MB VRAM)      22-bit (2 MB VRAM)      20-bit (2 MB VRAM)
  GP1(10h:index7)               N/A                     00000001h version       00000002h version
  GP1(10h:index8)               mirror of index0        00000000h zero          00000000h zero
  GP1(10h:index9..F)            mirror of index1..7     unknown                 N/A
  GP1(09h)                      N/A                     N/A                     VRAM size
  GP1(20h)                      N/A                     VRAM size/settings      N/A
  GP0(E1h).bit11                N/A                     N/A                     bit1 of texpage Y base
  GP0(E1h).bit12/13             without x/y-flip        without x/y-flip        with x/y-flip
  GP0(03h)                      N/A (no stored in fifo) unknown                 unknown/unused command
  Shaded Textures               ((color/8)*texel)/2     unknown                 (color*texel)/16
  GP0(02h) FillVram             xpos.bit0-3=0Fh=bugged  unknown                 xpos.bit0-3=ignored

  dma-to-vram: doesn't work with blksiz>10h (v2 gpu works with blksiz=8C0h!)
  dma-to-vram: MAYBE also needs extra software-handshake to confirm DMA done?
   320*224 pix = 11800h pix = 8C00h words
 */

const fn generate_rgb5_to_rgb8_table() -> [u8; 32] {
    let mut table = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        table[i] = ((i << 3) | (i >> 2)) as u8;
        i += 1;
    }
    table
}

static RGB5_TO_RGB8: [u8; 32] = generate_rgb5_to_rgb8_table();

pub const GPU_VERSION : u32 = 0;

#[derive(Debug, Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub m: bool,
}

/*
Color Attribute (Parameter for all Rendering commands, except Raw Texture)
  0-7    Red   (0..FFh)
  8-15   Green (0..FFh)
  16-23  Blue  (0..FFh)
  24-31  Command (in first paramter) (don't care in further parameters)
Caution: For untextured graphics, 8bit RGB values of FFh are brightest.
However, for modulation, 8bit values of 80h are brightest (values 81h..FFh are "brighter than bright" allowing to make textures about twice as bright as than they were originially stored in memory;
of course the results can't exceed the maximum brightness, ie. the 5bit values written to the framebuffer are saturated to max 1Fh).
 */
impl Color {
    pub const fn new(r: u8, g: u8, b: u8, m: bool) -> Color {
        Color { r, g, b, m }
    }

    pub fn from_u16(colour: u16) -> Color {
        let rb = (colour & 0x1F) as u8;
        let gb = ((colour >> 5) & 0x1F) as u8;
        let bb = ((colour >> 10) & 0x1F) as u8;

        let r = (rb << 3) | (rb >> 2);
        let g = (gb << 3) | (gb >> 2);
        let b = (bb << 3) | (bb >> 2);
        let a = (colour >> 15) != 0;

        Color::new(r, g, b, a)
    }

    pub fn from_u32(colour: u32) -> Color {
        let r = colour as u8;
        let g = (colour >> 8) as u8;
        let b = (colour >> 16) as u8;

        Color::new(r, g, b, false)
    }

    pub fn to_u16(&self) -> u16 {
        let mut pixel = 0;

        pixel |= ((self.r as u16) & 0xF8) >> 3;
        pixel |= ((self.g as u16) & 0xF8) << 2;
        pixel |= ((self.b as u16) & 0xF8) << 7;
        pixel |= (self.m as u16) << 15;

        pixel
    }

    fn mod_colors(texture_color: u8, shading_color: u8) -> u8 {
        cmp::min(255, u16::from(texture_color) * u16::from(shading_color) / 128) as u8
    }

    pub fn modulate_with(&self,c2:&Color) -> Color {
        Color::new(Color::mod_colors(self.r,c2.r),Color::mod_colors(self.g,c2.g),Color::mod_colors(self.b,c2.b),self.m)
    }

    pub fn dither(&self, dither_value: i8) -> Self {
        Self {
            r: self.r.saturating_add_signed(dither_value),
            g: self.g.saturating_add_signed(dither_value),
            b: self.b.saturating_add_signed(dither_value),
            m: self.m,
        }
    }

    pub fn r(&self) -> i32 {
        self.r as i32
    }

    pub fn g(&self) -> i32 {
        self.g as i32
    }

    pub fn b(&self) -> i32 {
        self.b as i32
    }
}

#[derive(Copy,Clone,Debug)]
enum TextureDepth {
    // 4 bits per pixel
    T4Bit,
    // 8 bits per pixel
    T8Bit,
    // 15 bits per pixel
    T15Bit,
    // same as T15Bit
    Reserved,
}

impl Default for TextureDepth {
    fn default() -> Self {
        TextureDepth::T4Bit
    }
}

#[derive(Default,Debug)]
struct Texture {
    /// Texture page base X coordinate (4 bits, 64 byte increment)
    page_base_x: u8,
    /// Texture page base Y coordinate (1bit, 256 line increment)
    page_base_y: u8,
    /// Texture page color depth
    depth: TextureDepth,
    /// Mirror textured rectangles along the x-axis
    rectangle_x_flip: bool,
    /// Mirror textured rectangles along the y-axis
    rectangle_y_flip: bool,
    /// Texture window x mask (8 pixel steps)
    window_x_mask: u8,
    /// Texture window y mask (8 pixel steps)
    window_y_mask: u8,
    /// Texture window x offset (8 pixel steps)
    window_x_offset: u8,
    /// Texture window y offset (8 pixel steps)
    window_y_offset: u8,
}

#[derive(Default,Debug)]
struct DrawingArea {
    /// Allow drawing to the display area
    draw_to_display: bool,
    /// Left-most column of drawing area
    area_left: u16,
    /// Top-most line of drawing area
    area_top: u16,
    /// Right-most column of drawing area
    area_right: u16,
    /// Bottom-most line of drawing area
    area_bottom: u16,
    /// Horizontal drawing offset applied to all vertex
    x_offset: i16,
    /// Vertical drawing offset applied to all vertex
    y_offset: i16,
}

impl DrawingArea {
    fn is_inside(&self,x:i16,y:i16) -> bool {
        x >= self.area_left as i16 && x <= self.area_right as i16 && y >= self.area_top as i16 && y <= self.area_bottom as i16
    }
}

#[derive(Default,Debug)]
struct DisplayConfig {
    /// First column of the display area in VRAM
    vram_x_start: u16,
    /// First line of the display area in VRAM
    vram_y_start: u16,
    /// Display output horizontal start relative to HSYNC
    horizontal_start: u16,
    /// Display output horizontal end relative to HSYNC
    horizontal_end: u16,
    /// Display output first line relative to VSYNC
    vertical_start: u16,
    /// Display output last line relative to VSYNC
    vertical_end: u16,
    /// Currently displayed field. For progressive output this is always Top.
    field: InterlaceField,
    /// Video output horizontal resolution
    h_res: VideoHorizontalResolution,
    /// Video output vertical resolution
    v_res: VideoVerticalResolution,
    /// Video mode
    video_mode: VideoMode,
    /// Display depth
    display_depth: DisplayDepth,
    display_disabled: bool,
    interlaced: bool,
}

impl DisplayConfig {
    fn visible_area(&self) -> (usize,usize) {
        let width = (((self.horizontal_end - self.horizontal_start) as f32) / self.h_res.get_divider() as f32) as usize - 1;
        let mut height = (self.vertical_end - self.vertical_start) as usize;
        if self.interlaced {
            height <<= 1;
        }
        // TODO check how to handle height limits
        // else if !self.interlaced && height > 240 {
        //     height = 240;
        // }
        (width,height)
    }
}

#[derive(Copy,Clone,Debug)]
struct VideoHorizontalResolution(usize);

impl VideoHorizontalResolution {
    /*
    GP1(08h) - Display mode
      0-1   Horizontal Resolution 1     (0=256, 1=320, 2=512, 3=640) ;GPUSTAT.17-18
      2     Vertical Resolution         (0=240, 1=480, when Bit5=1)  ;GPUSTAT.19
      3     Video Mode                  (0=NTSC/60Hz, 1=PAL/50Hz)    ;GPUSTAT.20
      4     Display Area Color Depth    (0=15bit, 1=24bit)           ;GPUSTAT.21
      5     Vertical Interlace          (0=Off, 1=On)                ;GPUSTAT.22
      6     Horizontal Resolution 2     (0=256/320/512/640, 1=368)   ;GPUSTAT.16
      7     "Reverseflag"               (0=Normal, 1=Distorted)      ;GPUSTAT.14
      8-23  Not used (zero)
     */
    fn from_gp1_08(mode:u32) -> Self {
        let hres = if ((mode >> 6) & 1) == 1 {
            368
        } else {
            match mode & 3 {
                0 => 256,
                1 => 320,
                2 => 512,
                3 => 640,
                _ => unreachable!()
            }
        };
        VideoHorizontalResolution(hres)
    }

    /*
     16    Horizontal Resolution 2     (0=256/320/512/640, 1=368)    ;GP1(08h).6
     17-18 Horizontal Resolution 1     (0=256, 1=320, 2=512, 3=640)  ;GP1(08h).0-1
     */
    fn to_status(&self) -> u32 {
        match self.0 {
            256 => 0 | 0 << 1,
            320 => 0 | 1 << 1,
            512 => 0 | 2 << 1,
            640 => 0 | 3 << 1,
            368 => 1,
            _ => unreachable!()
        }
    }

    /*
      320pix/PAL: 3406/8  = 425.75 dots     320pix/NTSC: 3413/8  = 426.625 dots
      640pix/PAL: 3406/4  = 851.5 dots      640pix/NTSC: 3413/4  = 853.25 dots
      256pix/PAL: 3406/10 = 340.6 dots      256pix/NTSC: 3413/10 = 341.3 dots
      512pix/PAL: 3406/5  = 681.2 dots      512pix/NTSC: 3413/5  = 682.6 dots
      368pix/PAL: 3406/7  = 486.5714 dots   368pix/NTSC: 3413/7  = 487.5714 dots
     */
    fn get_divider(&self) -> usize {
        match self.0 {
            256 => 10,
            320 => 8,
            512 => 5,
            640 => 4,
            368 => 7,
            _ => unreachable!()
        }
    }
}

impl Default for VideoHorizontalResolution {
    fn default() -> Self {
        VideoHorizontalResolution(320)
    }
}

#[derive(Copy,Clone,PartialEq,Debug)]
enum InterlaceField {
    Even,
    Odd
}

impl Default for InterlaceField {
    fn default() -> Self {
        InterlaceField::Odd
    }
}

#[derive(Copy,Clone,Debug)]
enum VideoVerticalResolution {
    Y240Lines,
    Y480Lines,
}

impl Default for VideoVerticalResolution {
    fn default() -> Self {
        VideoVerticalResolution::Y240Lines
    }
}

impl VideoVerticalResolution {
    fn total_lines(&self) -> usize {
        match self {
            VideoVerticalResolution::Y240Lines => 240,
            VideoVerticalResolution::Y480Lines => 480,
        }
    }
}

#[derive(Copy,Clone,Debug)]
enum VideoMode {
    Ntsc,
    Pal
}

impl VideoMode {
    /*
      263 scanlines per field for NTSC non-interlaced
      262.5 scanlines per field for NTSC interlaced

      314 scanlines per field for PAL non-interlaced
      312.5 scanlines per field for PAL interlaced
     */
    #[inline]
    fn total_lines(&self) -> usize {
        match self {
            VideoMode::Ntsc => 263,
            VideoMode::Pal => 314
        }
    }
    #[inline]
    fn video_clock(&self) -> usize {
        match self {
            VideoMode::Ntsc => 53_693_175,
            VideoMode::Pal => 53_203_425
        }
    }
    #[inline]
    fn horizontal_cycles(&self) -> usize {
        match self {
            VideoMode::Ntsc => 3413,
            VideoMode::Pal => 3406
        }
    }
    #[inline]
    fn frame_micros(&self) -> u64 {
        match self {
            VideoMode::Ntsc => (1_000_000.0f64 / 60.0).floor() as u64,
            VideoMode::Pal => 1_000_000 / 50
        }
    }
}

impl Default for VideoMode {
    fn default() -> Self {
        VideoMode::Ntsc
    }
}

#[derive(Copy,Clone,Debug)]
enum DisplayDepth {
    /// 15 bits per pixel
    D15Bits,
    /// 24 bit per pixel
    D24Bits,
}

impl Default for DisplayDepth {
    fn default() -> Self {
        DisplayDepth::D15Bits
    }
}

#[derive(Copy,Clone,Debug)]
enum DMADirection {
    Off,
    Fifo,
    CpuToGp0,
    VRamToCpu,
}

impl Default for DMADirection {
    fn default() -> Self {
        DMADirection::Off
    }
}

#[derive(Default)]
struct ReadyBits {
    ready_to_receive_cmd_word: bool, // 26
    ready_to_send_vram_to_cpu: bool, // 27
    ready_to_receive_dma_block: bool, // 28
}

#[derive(Debug,Clone,Copy,Default)]
enum SemiTransparency {
    #[default]
    Average,
    Additive,
    Subtractive,
    AddQuarter,
}

impl SemiTransparency {
    // GP0(E1) 5-6   Semi-transparency     (0=B/2+F/2, 1=B+F, 2=B-F, 3=B+F/4)   ;GPUSTAT.5-6
    fn from_command(cmd:u32) -> Self {
        match (cmd >> 5) & 3 {
            0b00 => SemiTransparency::Average,
            0b01 => SemiTransparency::Additive,
            0b10 => SemiTransparency::Subtractive,
            0b11 => SemiTransparency::AddQuarter,
            _ => unreachable!()
        }
    }
    
    fn to_status(&self) -> u32 {
        match self {
            SemiTransparency::Average => 0b00,
            SemiTransparency::Additive => 0b01,
            SemiTransparency::Subtractive => 0b10,
            SemiTransparency::AddQuarter => 0b11,
        }
    }

    fn blend_rgb555(&self, fg_rgb555: u16, bg_rgb555: u16) -> u16 {
        let fg_r = (fg_rgb555 & 0x1F) as u8;
        let fg_g = ((fg_rgb555 >> 5) & 0x1F) as u8;
        let fg_b = ((fg_rgb555 >> 10) & 0x1F) as u8;

        let bg_r = (bg_rgb555 & 0x1F) as u8;
        let bg_g = ((bg_rgb555 >> 5) & 0x1F) as u8;
        let bg_b = ((bg_rgb555 >> 10) & 0x1F) as u8;

        let (r, g, b) = match self {
            SemiTransparency::Average => (
                (bg_r + fg_r) >> 1,
                (bg_g + fg_g) >> 1,
                (bg_b + fg_b) >> 1,
            ),
            SemiTransparency::Additive => (
                (bg_r + fg_r).min(31),
                (bg_g + fg_g).min(31),
                (bg_b + fg_b).min(31),
            ),
            SemiTransparency::Subtractive => (
                bg_r.saturating_sub(fg_r),
                bg_g.saturating_sub(fg_g),
                bg_b.saturating_sub(fg_b),
            ),
            SemiTransparency::AddQuarter => (
                (bg_r + (fg_r >> 2)).min(31),
                (bg_g + (fg_g >> 2)).min(31),
                (bg_b + (fg_b >> 2)).min(31),
            ),
        };

        ((b as u16) << 10) | ((g as u16) << 5) | (r as u16)
    }
}

#[derive(Default,Debug)]
pub struct CommandFifo {
    buf: [u32; 16],
    head: u8,
    tail: u8,
    len:  u8,
}

impl CommandFifo {
    /// Push a word
    /// Returns false if it's full.
    #[inline(always)]
    pub fn push(&mut self, value: u32) -> bool {
        if self.len == 16 {
            return false;
        }

        self.buf[self.tail as usize] = value;
        self.tail = (self.tail + 1) & 0x0F;
        self.len += 1;
        true
    }

    /// Pop a word.
    /// None if it's empty.
    #[inline(always)]
    pub fn pop(&mut self) -> Option<u32> {
        if self.len == 0 {
            return None;
        }

        let v = self.buf[self.head as usize];
        self.head = (self.head + 1) & 0x0F;
        self.len -= 1;
        Some(v)
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.len == 16
    }

    #[inline(always)]
    pub fn len(&self) -> u8 {
        self.len
    }

    /// Peek a word
    #[inline(always)]
    pub fn peek(&self) -> Option<u32> {
        if self.len == 0 {
            None
        } else {
            Some(self.buf[self.head as usize])
        }
    }

    /// Reset
    #[inline(always)]
    pub fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.len  = 0;
    }
}

#[derive(Default)]
struct Raster {
    total_lines: usize,
    total_cycles: usize,
    cycles: usize,
    v_blank: bool,
    h_blank: bool,
    raster_line: usize,
}

type GP0Operation = fn(&mut GPU,u32,&mut IrqHandler);

#[derive(Debug,Default,Clone,Copy)]
struct VRamCopyConfig {
    coord_x: u16,
    coord_y: u16,
    counter_x: u16,
    counter_y: u16,
    width: u16,
    height: u16,
}

impl VRamCopyConfig {
    fn next(&self) -> Option<VRamCopyConfig> {
        let mut counter_x = self.counter_x.wrapping_add(2); // each step processes 2 halfword
        let mut counter_y = self.counter_y;
        let mut finished = false;
        if counter_x >= self.width {
            counter_x = 0;
            counter_y += 1;
            finished =  counter_y == self.height;
        }
        if finished {
            None
        }
        else {
            Some(Self {
                coord_x: self.coord_x,
                coord_y: self.coord_y,
                counter_x,
                counter_y,
                width: self.width,
                height: self.height,
            })
        }
    }
}

#[derive(Debug)]
enum Gp0State {
    WaitingCommand,
    WaitingCommandParameters(GP0Operation,Option<usize>),
    VRamCopy(GP0Operation,VRamCopyConfig),
    WaitingPolyline(GP0Operation,usize,Vertex,Color,bool,bool), // arguments,start vertex, color, is_shaded, semi_transparency
}
/*
Vertex (Parameter for Polygon, Line, Rectangle commands)
  0-10   X-coordinate (signed, -1024..+1023)
  11-15  Not used (usually sign-extension, but ignored by hardware)
  16-26  Y-coordinate (signed, -1024..+1023)
  26-31  Not used (usually sign-extension, but ignored by hardware)
Size Restriction: The maximum distance between two vertices is 1023 horizontally, and 511 vertically. Polygons and lines that are exceeding that dimensions are NOT rendered. For example, a line from Y1=-300 to Y2=+300 is NOT rendered, a line from Y1=-100 to Y2=+400 is rendered (as far as it is within the drawing area).
If portions of the polygon/line/rectangle are located outside of the drawing area, then the hardware renders only the portion that is inside of the drawing area.
 */
#[derive(Debug,Copy,Clone)]
struct Vertex {
    pub x: i16,
    pub y: i16,
}

impl Vertex {
    fn from_command_parameter(cmd:u32) -> Self {
        let x = (((cmd & 0x7FF) << 5) as i16) >> 5;
        let y = ((((cmd >> 16) & 0x7FF) << 5) as i16) >> 5;
        Self { x, y }
    }
    #[inline]
    fn add_offset(&mut self,x_offset:i16, y_offset:i16) {
        self.x += x_offset;
        self.y += y_offset;
    }
    #[inline]
    fn is_inside_drawing_area(&self,drawing_area: &DrawingArea) -> bool {
        drawing_area.is_inside(self.x,self.y)
    }
    #[inline]
    fn dx(&self,other:&Vertex) -> i16 {
        other.x - self.x
    }
    #[inline]
    fn dy(&self,other:&Vertex) -> i16 {
        other.y - self.y
    }
}

pub struct GPU {
    renderer: Box<dyn Renderer>,
    vram: Vec<u8>, // little endian format
    gp1_commands: [fn (&mut GPU,u32);0x100],
    cmd_fifo: CommandFifo,
    texture: Texture,
    semi_transparency: SemiTransparency,
    /// Enable dithering from 24 to 15bits RGB
    dithering: bool,
    /// Force "mask" bit of the pixel to 1 when writing to VRAM (otherwise don't modify it)
    force_set_mask_bit: bool,
    /// Don't draw to pixels which have the "mask" bit set
    preserve_masked_pixels: bool,
    drawing_area: DrawingArea,
    display_config: DisplayConfig,
    reverse_flag: bool,
    /// True when the interrupt is active
    irq: bool,
    dma_direction: DMADirection,
    ready_bits: ReadyBits,
    raster: Raster,
    dot_clock_cycles: usize,
    gpu_read_register: u32,
    gp0state: Gp0State,
    show_whole_vram: bool,
}

impl GPU {
    pub fn new(renderer:Box<dyn Renderer>) -> Self {
        let mut gpu = GPU {
            renderer,
            vram: vec![0;1024 * 512 * 2],//.into_boxed_slice(),
            gp1_commands: [GPU::gp1_not_implemented;0x100],
            cmd_fifo: CommandFifo::default(),
            texture: Texture::default(),
            semi_transparency: SemiTransparency::Average,
            dithering: false,
            force_set_mask_bit: false,
            preserve_masked_pixels: false,
            drawing_area: DrawingArea::default(),
            display_config: DisplayConfig::default(),
            reverse_flag: false,
            irq: false,
            dma_direction: DMADirection::default(),
            ready_bits: ReadyBits::default(),
            raster: Raster::default(),
            dot_clock_cycles: 0,
            gpu_read_register: 0,
            gp0state: Gp0State::WaitingCommand,
            show_whole_vram: false,
        };

        gpu.display_config.horizontal_start = 0x260 + 0;
        gpu.display_config.horizontal_end = 0x260 + 320 * 8;
        gpu.display_config.vertical_start = 0x88 - 240 / 2;
        gpu.display_config.vertical_end = 0x88 + 240 / 2;

        gpu.raster.total_lines = gpu.display_config.video_mode.total_lines();
        gpu.raster.total_cycles = gpu.display_config.video_mode.horizontal_cycles();

        gpu.display_config.display_disabled = true;
        gpu.ready_bits.ready_to_receive_cmd_word = true;
        gpu.init_gp1_commands();

        //gpu.ready_bits.ready_to_send_vram_to_cpu = true;
        gpu.ready_bits.ready_to_receive_dma_block = true;

        gpu
    }

    pub fn set_show_vram(&mut self,enabled:bool) {
        self.show_whole_vram = enabled;
    }

    pub fn get_renderer_mut(&mut self) -> &mut Box<dyn Renderer> {
        &mut self.renderer
    }
    /*
    1F801814h - GPUSTAT - GPU Status Register (R)
      0-3   Texture page X Base   (N*64)                              ;GP0(E1h).0-3
      4     Texture page Y Base   (N*256) (ie. 0 or 256)              ;GP0(E1h).4
      5-6   Semi Transparency     (0=B/2+F/2, 1=B+F, 2=B-F, 3=B+F/4)  ;GP0(E1h).5-6
      7-8   Texture page colors   (0=4bit, 1=8bit, 2=15bit, 3=Reserved)GP0(E1h).7-8
      9     Dither 24bit to 15bit (0=Off/strip LSBs, 1=Dither Enabled);GP0(E1h).9
      10    Drawing to display area (0=Prohibited, 1=Allowed)         ;GP0(E1h).10
      11    Set Mask-bit when drawing pixels (0=No, 1=Yes/Mask)       ;GP0(E6h).0
      12    Draw Pixels           (0=Always, 1=Not to Masked areas)   ;GP0(E6h).1
      13    Interlace Field       (or, always 1 when GP1(08h).5=0)
      14    "Reverseflag"         (0=Normal, 1=Distorted)             ;GP1(08h).7
      15    Texture Disable       (0=Normal, 1=Disable Textures)      ;GP0(E1h).11
      16    Horizontal Resolution 2     (0=256/320/512/640, 1=368)    ;GP1(08h).6
      17-18 Horizontal Resolution 1     (0=256, 1=320, 2=512, 3=640)  ;GP1(08h).0-1
      19    Vertical Resolution         (0=240, 1=480, when Bit22=1)  ;GP1(08h).2
      20    Video Mode                  (0=NTSC/60Hz, 1=PAL/50Hz)     ;GP1(08h).3
      21    Display Area Color Depth    (0=15bit, 1=24bit)            ;GP1(08h).4
      22    Vertical Interlace          (0=Off, 1=On)                 ;GP1(08h).5
      23    Display Enable              (0=Enabled, 1=Disabled)       ;GP1(03h).0
      24    Interrupt Request (IRQ1)    (0=Off, 1=IRQ)       ;GP0(1Fh)/GP1(02h)
      25    DMA / Data Request, meaning depends on GP1(04h) DMA Direction:
              When GP1(04h)=0 ---> Always zero (0)
              When GP1(04h)=1 ---> FIFO State  (0=Full, 1=Not Full)
              When GP1(04h)=2 ---> Same as GPUSTAT.28
              When GP1(04h)=3 ---> Same as GPUSTAT.27
      26    Ready to receive Cmd Word   (0=No, 1=Ready)  ;GP0(...) ;via GP0
      27    Ready to send VRAM to CPU   (0=No, 1=Ready)  ;GP0(C0h) ;via GPUREAD
      28    Ready to receive DMA Block  (0=No, 1=Ready)  ;GP0(...) ;via GP0
      29-30 DMA Direction (0=Off, 1=?, 2=CPUtoGP0, 3=GPUREADtoCPU)    ;GP1(04h).0-1
      31    Drawing even/odd lines in interlace mode (0=Even or Vblank, 1=Odd)

      Ready Bits
        Bit28: Normally, this bit gets cleared when the command execution is busy (ie. once when the command and all of its parameters are received), however, for Polygon and Line Rendering commands, the bit gets cleared immediately after receiving the command word (ie. before receiving the vertex parameters). The bit is used as DMA request in DMA Mode 2, accordingly, the DMA would probably hang if the Polygon/Line parameters are transferred in a separate DMA block (ie. the DMA probably starts ONLY on command words).
        Bit27: Gets set after sending GP0(C0h) and its parameters, and stays set until all data words are received; used as DMA request in DMA Mode 3.
        Bit26: Gets set when the GPU wants to receive a command. If the bit is cleared, then the GPU does either want to receive data, or it is busy with a command execution (and doesn't want to receive anything).
        Bit25: This is the DMA Request bit, however, the bit is also useful for non-DMA transfers, especially in the FIFO State mode.
     */
    pub fn gpu_stat_read(&self) -> u32 {
        let mut st = 0;

        st |= self.texture.page_base_x as u32;
        st |= ((self.texture.page_base_y & 1) as u32) << 4;
        st |= self.semi_transparency.to_status() << 5;
        st |= (self.texture.depth as u32) << 7;
        st |= (self.dithering as u32) << 9;
        st |= (self.drawing_area.draw_to_display as u32) << 10;
        st |= (self.force_set_mask_bit as u32) << 11;
        st |= (self.preserve_masked_pixels as u32) << 12;
        st |= (self.display_config.field as u32) << 13;
        st |= (self.reverse_flag as u32) << 14;
        st |= 0 << 15; // forced
        st |= self.display_config.h_res.to_status() << 16;
        st |= (self.display_config.v_res as u32) << 19;
        st |= (self.display_config.video_mode as u32) << 20;
        st |= (self.display_config.display_depth as u32) << 21;
        st |= (self.display_config.interlaced as u32) << 22;
        st |= (self.display_config.display_disabled as u32) << 23;
        st |= (self.irq as u32) << 24;
        let dma = match self.dma_direction {
            DMADirection::Off => 0,
            DMADirection::Fifo => (!self.cmd_fifo.is_full()) as u32,
            DMADirection::CpuToGp0 => self.ready_bits.ready_to_receive_dma_block as u32, // same as 28
            DMADirection::VRamToCpu => self.ready_bits.ready_to_receive_cmd_word as u32, // same as 27
        };
        st |= dma << 25;
        let ready_receive_cmd_word = matches!(self.gp0state,Gp0State::WaitingCommand);
        st |= (ready_receive_cmd_word as u32) << 26;
        st |= (self.ready_bits.ready_to_send_vram_to_cpu as u32) << 27;
        st |= (self.ready_bits.ready_to_receive_dma_block as u32) << 28;
        st |= (self.dma_direction as u32) << 29;

        let mut bit31 = 1;
        if self.display_config.field == InterlaceField::Even || self.raster.v_blank {
            bit31 = 0;
        }

        st |= bit31 << 31;

        //info!("Reading GPUSTAT: {:08X} {}",st,self.display_config.h_res.to_status());
        st
    }

    pub fn gpu_read_read(&mut self) -> u32 {
        match self.gp0state {
            Gp0State::VRamCopy(operation,config) => {
                let vram_x = config.coord_x + config.counter_x;
                let vram_y = config.coord_y + config.counter_y;
                self.gpu_read_register = self.get_pixel_15(self.get_vram_offset_15(vram_x, vram_y)) as u32 | (self.get_pixel_15(self.get_vram_offset_15(vram_x + 1, vram_y)) as u32) << 16;
                debug!("VRam->Cpu ({vram_x},{vram_y}) = {:08X}",self.gpu_read_register);
                match config.next() {
                    Some(next) => {
                        self.gp0state = Gp0State::VRamCopy(operation,next)
                    }
                    None => {
                        if (config.width & 1) == 1 {
                            self.gpu_read_register &= 0xFFFF; // remove upper halfword if width is odd
                        }
                        debug!("VRam->Cpu operation terminated.");
                        self.ready_bits.ready_to_send_vram_to_cpu = false;
                        self.gp0state = Gp0State::WaitingCommand;
                    }
                }
            },
            _ => {}
        }
        self.gpu_read_register
    }

    pub fn gpu_read_peek(&self) -> u32 {
        self.gpu_read_register
    }

    pub fn send_first_hblank_event(&self,clock:&mut Clock) {
        clock.schedule_gpu(EventType::HBlankEnd,self.display_config.horizontal_start as u64);
    }

    pub fn on_hblank_end(&mut self,over_cycles:usize,bus:&mut Bus) {
        bus.get_clock_mut().schedule_gpu(EventType::HBlankStart,self.display_config.horizontal_end as u64 - self.display_config.horizontal_start as u64 - over_cycles as u64);
        self.raster.h_blank = false;
        // sync timer0 we exited hblank
        let (timer0,clock) = bus.get_timer0_and_clock_mut();
        timer0.on_blank_end(clock);
    }
    pub fn on_hblank_start(&mut self,bus:&mut Bus,irq_handler: &mut IrqHandler,over_cycles:usize) {
        bus.get_clock_mut().schedule_gpu(EventType::RasterLineEnd,self.raster.total_cycles as u64 - self.display_config.horizontal_end as u64 - over_cycles as u64);
        self.raster.h_blank = true;
        // sync timer0 we entered hblank
        let (timer0,clock) = bus.get_timer0_and_clock_mut();
        timer0.on_blank_start(clock,self.display_config.h_res.get_divider());
        // clock timer1 for hblank source
        bus.get_timer1_mut().cycle_hblank_clock(irq_handler);
    }
    pub fn on_raster_line_end(&mut self,bus:&mut Bus,irq_handler: &mut IrqHandler,over_cycles:usize) -> bool {
        let mut new_frame = false;
        bus.get_clock_mut().schedule_gpu(EventType::HBlankEnd,self.display_config.horizontal_start as u64 - over_cycles as u64);
        self.raster.raster_line += 1;
        // check vblank
        if self.raster.raster_line == self.display_config.vertical_start as usize { // start of drawing area
            self.raster.v_blank = false;
            // sync timer1 we exited vblank
            let (timer1,clock) = bus.get_timer1_and_clock_mut();
            timer1.on_blank_end(clock);
        }
        else if self.raster.raster_line == self.display_config.vertical_end as usize { // end of drawing area, entering in vblank
            self.raster.v_blank = true;
            self.set_vblank(bus,irq_handler);
            new_frame = true;
        }

        // check end of frame
        if self.raster.raster_line >= self.raster.total_lines {
            if !self.raster.v_blank { // if vblank has not been set, force it at the end of the frame
                self.set_vblank(bus,irq_handler);
                new_frame = true;
            }
            self.raster.raster_line = 0;
            if self.display_config.interlaced {
                // next field
                self.display_config.field = match self.display_config.field {
                    InterlaceField::Even => InterlaceField::Odd,
                    InterlaceField::Odd => InterlaceField::Even,
                }
            }
        }

        new_frame
    }

    fn set_vblank(&mut self,bus:&mut Bus,irq_handler: &mut IrqHandler) {
        self.raster.v_blank = true;
        // sync timer1 we entered vblank
        let (timer1,clock) = bus.get_timer1_and_clock_mut();
        timer1.on_blank_start(clock,self.display_config.h_res.get_divider());
        // trigger vblank irq
        irq_handler.set_irq(InterruptType::VBlank);
        // new frame
        self.generate_new_frame();
    }

    fn generate_new_frame(&mut self) {
        let (frame_width,frame_height) = if self.show_whole_vram {
            (1024,512)
        }
        else {
            self.display_config.visible_area()
        };

        let crt_width = if self.show_whole_vram { 1024 } else { self.display_config.h_res.0 };
        let crt_height = if self.show_whole_vram {
            512
        }
        else {
            if self.display_config.interlaced {
                self.display_config.video_mode.total_lines() << 1
            } else {
                self.display_config.video_mode.total_lines()
            }
        };

        let crt_start_x_offset = ((crt_width as u16).saturating_sub(frame_width as u16) >> 1) as usize;
        let crt_start_y_offset = ((crt_height as u16).saturating_sub(frame_height as u16) >> 1) as usize;

        let mut frame_buffer = vec![0u8; crt_width * crt_height << 2]; // RGBA8
        if !self.display_config.display_disabled {
            let vram_x0 = self.display_config.vram_x_start as usize;
            let vram_y0 = self.display_config.vram_y_start as usize;
            let is24_bit = matches!(self.display_config.display_depth,DisplayDepth::D24Bits);
            for y in 0..frame_height {
                let mut row_offset = (crt_start_y_offset + y) * (crt_width << 2) + (crt_start_x_offset << 2);
                for x in 0..frame_width {
                    let vram_x = vram_x0 + x;
                    let vram_y = vram_y0 + y;

                    let (r, g, b) = if is24_bit {
                        let byte_offset = self.get_vram_offset_24(vram_x as u16, vram_y as u16);
                        if byte_offset + 2 < self.vram.len() {
                            let r = self.vram[byte_offset];
                            let g = self.vram[byte_offset + 1];
                            let b = self.vram[byte_offset + 2];
                            (r, g, b)
                        } else {
                            continue;
                        }
                    } else {
                        let pixel = self.get_pixel_15(self.get_vram_offset_15(vram_x as u16, vram_y as u16));

                        let r = RGB5_TO_RGB8[(pixel & 0x1F) as usize];
                        let g = RGB5_TO_RGB8[((pixel >> 5) & 0x1F) as usize];
                        let b = RGB5_TO_RGB8[((pixel >> 10) & 0x1F) as usize];
                        (r, g, b)
                    };

                    frame_buffer[row_offset] = r;
                    frame_buffer[row_offset + 1] = g;
                    frame_buffer[row_offset + 2] = b;
                    frame_buffer[row_offset + 3] = 0xFF;
                    row_offset += 4;
                }
            }
        }

        self.renderer.render_frame(GPUFrameBuffer::new(Arc::new(frame_buffer),crt_width,crt_height,frame_width,frame_height));
    }
}

impl DmaDevice for GPU {
    fn is_dma_ready(&self) -> bool {
        true
    }
    fn dma_request(&self) -> bool {
        true
    }
    fn dma_write(&mut self, word: u32,clock:&mut Clock,irq_handler:&mut  IrqHandler) {
        self.gp0_cmd(word,clock,irq_handler);
    }
    fn dma_read(&mut self) -> u32 {
        self.gpu_read_read()
    }
}