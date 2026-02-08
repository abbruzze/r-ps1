
pub(super) struct GPUTimings {}

impl GPUTimings {
    const POLY_LINE_PENALTY : usize = 15;

    pub fn rectangle(width:u16,height:u16,is_textured:bool,is_semi_transparent:bool) -> usize {
        let area = width * height;
        let (base_cycles,mut pixel_cycles) = if is_textured {
            (30,2)
        }
        else {
            (20,1)
        };
        if is_semi_transparent {
            pixel_cycles += 1;
        };

        base_cycles + area as usize * pixel_cycles
    }

    pub fn rectangle_fill(width:u16,height:u16) -> usize {
        width as usize * height as usize * 1
    }

    pub fn line(pixels:usize, is_gouraud:bool, is_semi_transparent:bool) -> usize {
        let mut pixel_cycles = 1usize;
        if is_gouraud {
            pixel_cycles += 1;
        }
        if is_semi_transparent {
            pixel_cycles += 1;
        }

        20 + pixels * pixel_cycles
    }

    pub fn triangle(pixels:usize,is_gouraud:bool, is_semi_transparent:bool,is_texture:bool) -> usize {
        let mut base_cycles = 30;
        let mut pixel_cycles = 1;
        if is_texture {
            base_cycles += 10;
            pixel_cycles += 1;
        }
        if is_gouraud {
            base_cycles += 5;
            pixel_cycles += 1;
        }
        if is_semi_transparent {
            pixel_cycles += 1;
        }

        base_cycles + pixel_cycles * pixels
    }

    pub fn vram_to_vram_copy(width:usize,height:usize) -> usize {
        30 + ((width * height) >> 1)
    }
}