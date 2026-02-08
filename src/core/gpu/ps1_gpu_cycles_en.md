# PlayStation 1 GPU Cycles Table - Graphics Primitives

## General Notes
- PS1 GPU operates at 53.222.400 Hz (NTSC) or 53.693.175 Hz (PAL)
- Cycle counts are approximate and depend on various factors
- Rendering is limited to approximately 1,000,000 pixels/sec for textured operations

---

## 1. POLYGON PRIMITIVES

### Flat-Shaded Triangles (Solid Color)

| Type | Gouraud | Texture | Semi-transp. | Base Cycles | Cycles/Pixel | Notes |
|------|---------|---------|--------------|-------------|--------------|-------|
| Flat Triangle | No | No | No | ~30 | 1 | Fastest |
| Flat Triangle | No | No | Yes | ~30 | 2 | +1 cycle/pixel for blending |
| Flat Triangle | No | Yes | No | ~40 | 2-3 | Depends on texture cache |
| Flat Triangle | No | Yes | Yes | ~40 | 4-5 | Texture + blending |

### Gouraud-Shaded Triangles (Interpolated)

| Type | Gouraud | Texture | Semi-transp. | Base Cycles | Cycles/Pixel | Notes |
|------|---------|---------|--------------|-------------|--------------|-------|
| Gouraud Triangle | Yes | No | No | ~35 | 1-2 | Color interpolation |
| Gouraud Triangle | Yes | No | Yes | ~35 | 3 | Gouraud + blending |
| Gouraud Triangle | Yes | Yes | No | ~45 | 3-4 | Texture + Gouraud |
| Gouraud Triangle | Yes | Yes | Yes | ~45 | 5-6 | Everything enabled |

### Quadrilaterals (Quads)

| Type | Gouraud | Texture | Semi-transp. | Base Cycles | Cycles/Pixel | Notes |
|------|---------|---------|--------------|-------------|--------------|-------|
| Flat Quad | No | No | No | ~50 | 1 | Rendered as 2 triangles |
| Flat Quad | No | Yes | No | ~65 | 2-3 | 2 textured triangles |
| Gouraud Quad | Yes | No | No | ~60 | 1-2 | 2 Gouraud triangles |
| Gouraud Quad | Yes | Yes | No | ~75 | 3-4 | More expensive setup |
| Gouraud Quad | Yes | Yes | Yes | ~75 | 5-6 | Maximum cost |

---

## 2. LINE PRIMITIVES

### Lines

| Type | Gouraud | Semi-transp. | Base Cycles | Cycles/Pixel | Notes |
|------|---------|--------------|-------------|--------------|-------|
| Flat Line | No | No | ~20 | 1 | Single color |
| Flat Line | No | Yes | ~20 | 2 | With blending |
| Gouraud Line | Yes | No | ~25 | 1-2 | Color interpolation |
| Gouraud Line | Yes | Yes | ~25 | 2-3 | Gouraud + blending |
| Polyline | - | - | ~15/vertex | - | + single line cost |

---

## 3. SPRITES AND RECTANGLES

### Non-Textured Sprites (Solid Color)

| Size | Semi-transp. | Base Cycles | Cycles/Pixel | Notes |
|------|--------------|-------------|--------------|-------|
| Sprite 8x8 | No | ~15 | 1 | Very fast, fill only |
| Sprite 8x8 | Yes | ~15 | 2 | + blending |
| Sprite 16x16 | No | ~18 | 1 | Common size |
| Sprite 16x16 | Yes | ~18 | 2 | With blending |
| Sprite 32x32 | No | ~20 | 1 | Cache-friendly |
| Sprite 32x32 | Yes | ~20 | 2 | + blending |
| Sprite 64x64+ | No | ~25 | 1 | Large solid sprites |
| Sprite 64x64+ | Yes | ~25 | 2 | Blending overhead |
| Variable Sprite | - | ~15-30 | 1-2 | Depends on size |

### Textured Sprites

| Size | Semi-transp. | Base Cycles | Cycles/Pixel | Notes |
|------|--------------|-------------|--------------|-------|
| Sprite 8x8 | No | ~25 | 2 | Small, cache-friendly |
| Sprite 8x8 | Yes | ~25 | 3 | + blending |
| Sprite 16x16 | No | ~30 | 2 | Common size |
| Sprite 16x16 | Yes | ~30 | 3 | Frequent use |
| Sprite 32x32 | No | ~35 | 2-3 | Possible cache misses |
| Sprite 32x32 | Yes | ~35 | 4 | + blending overhead |
| Sprite 64x64+ | No | ~40 | 2-3 | Large sprites |
| Sprite 64x64+ | Yes | ~40 | 4-5 | Maximum overhead |
| Variable Sprite | - | ~30-45 | 2-5 | Depends on size |

### Rectangles (Fill)

| Type | Size | Base Cycles | Cycles/Pixel | Notes |
|------|------|-------------|--------------|-------|
| Rect Fill | Small (<32px) | ~20 | 0.5-1 | Very fast |
| Rect Fill | Medium (32-128px) | ~25 | 1 | Common operation |
| Rect Fill | Large (>128px) | ~30 | 1 | Clearscreen, etc |
| Rect Fill | Full Screen | ~35 | 1 | 320x240 = 76800 pixels |

**Notes on PS1 Sprites:**
- **Non-textured sprites** are essentially rectangles filled with a solid color
- **Textured sprites** load pixels from a texture page in VRAM
- Main difference is texture fetch (+1 cycle/pixel)
- Sprites can have variable sizes (not just powers of 2)
- GPU commands for sprites are: SPRT (free size), SPRT_8, SPRT_16 (fixed size)

---

## 4. SPECIAL OPERATIONS

### Texture Blitting and VRAM Transfers

| Operation | Base Cycles | Throughput | Notes |
|-----------|-------------|------------|-------|
| VRAM→CPU | ~50 | ~1 pixel/cycle | VRAM read |
| CPU→VRAM | ~40 | ~1 pixel/cycle | VRAM write |
| VRAM→VRAM | ~30 | ~2 pixels/cycle | Internal copy, faster |
| Load CLUT | ~100 | - | Load 16/256 color palette |
| Load Texture Page | ~150 | - | Texture cache setup |

### Framebuffer Operations

| Operation | Cycles | Notes |
|-----------|--------|-------|
| Clear Screen | ~80000 | 320x240 @ 1 cycle/pixel |
| VSync Wait | 0 | CPU waits, GPU continues |
| Double Buffer Swap | ~50 | Switch display/draw buffer |
| Scissor Setup | ~10 | Set rendering area |

---

## 5. ADDITIONAL OVERHEAD PER PIXEL

### Blending Modes (Semi-transparency)

| Mode | Additional Cycles/Pixel | Notes |
|------|-------------------------|-------|
| 0.5 Background + 0.5 Foreground | +1 | Standard mode - read, blend, write |
| Background + Foreground | +1 | Additive blending |
| Background - Foreground | +1 | Subtractive blending |
| Background + 0.25 Foreground | +1 | Brightness blending |

**All blending modes add ~1 cycle/pixel** because they require:
1. Read background pixel from framebuffer
2. Blending operation (add/sub/average)
3. Write result

### Texture Filtering and Sampling

| Parameter | Cycle Impact | Notes |
|-----------|--------------|-------|
| Nearest (Point) | Baseline | No filtering (PS1 standard) |
| Texture Cache Hit | +0 | Data already in cache |
| Texture Cache Miss | +0.5-1 cycle/pixel | Fetch from VRAM, not multiplicative |
| Texture Page Change | +150 cycles (one-time) | Texture page switch |
| CLUT Change | +100 cycles (one-time) | Palette change |

**Note**: PS1 has NO hardware texture filtering - always uses nearest neighbor

### Area/Pixel Size

| Rendered Area | Impact | Notes |
|---------------|--------|-------|
| < 100 pixels | Negligible | Setup dominates |
| 100-1000 pixels | Linear | Cycles/pixel matters |
| > 1000 pixels | Linear | Bandwidth limit |
| Screen Fill | ~80000 cycles | Full 320x240 |

---

## 6. CYCLE CALCULATION FORMULA

### How Calculation Works

Cycles are **NOT multiplicative** but **additive**:

```
Cycles_Per_Pixel = Base_Rendering_Cycles 
                   + Texture_Cycles (if textured)
                   + Gouraud_Cycles (if interpolated shading)
                   + Blending_Cycles (if semi-transparent)
```

### Detailed Per-Pixel Breakdown

| Operation | Cycles/Pixel | When Applied |
|-----------|--------------|--------------|
| Write Pixel (base) | 1 | Always |
| Texture Fetch | +1 | If textured |
| Gouraud Interpolation | +0-1 | If Gouraud shading (minimal overhead) |
| Blending (read + blend) | +1 | If semi-transparent |
| Texture Cache Miss | +0.5 | Occasional, not always |

### Concrete Examples

#### Flat Non-Textured Triangle
```
Cycles/pixel = 1 (write)
Total = 30 (setup) + (area × 1)
```

#### Flat Textured Triangle
```
Cycles/pixel = 1 (write) + 1 (texture fetch) = 2
Total = 40 (setup) + (area × 2)
```

#### Flat Textured + Semi-transparent Triangle
```
Cycles/pixel = 1 (write) + 1 (texture) + 1 (blend) = 3
Total = 40 (setup) + (area × 3)
```

#### Gouraud Textured + Semi-transparent Triangle
```
Cycles/pixel = 1 (write) + 1 (texture) + 0.5 (gouraud) + 1 (blend) ≈ 3.5
Total = 45 (setup) + (area × 3.5)
```

**Important**: Gouraud shading has very low per-pixel overhead because interpolation is hardware-accelerated.

---

## 7. CALCULATION FORMULAS

### Total Cycles per Primitive

```
Total_Cycles = Base_Cycles + (Pixel_Area × Cycles_Per_Pixel)

Where:
- Base_Cycles = Primitive setup (command, vertices, attributes)
- Pixel_Area = Number of pixels actually rendered
- Cycles_Per_Pixel = Depends on texture, shading, blending
```

### Practical Examples

#### Flat Textured Triangle (500 pixel area)
```
Cycles = 40 (base) + (500 × 2.5) = 1290 cycles ≈ 24.2 µs @ 53.22 MHz
```

#### Semi-transparent 32x32 Sprite
```
Cycles = 35 (base) + (1024 × 4) = 4131 cycles ≈ 77.6 µs
```

#### Gouraud+Texture+Blending Quad (800 pixel area)
```
Cycles = 75 (base) + (800 × 5.5) = 4475 cycles ≈ 84.1 µs
```

---

## 8. HARDWARE LIMITS

| Limit | Value | Notes |
|-------|-------|-------|
| Max Pixels/Frame (60fps) | ~850000 | @53.22MHz, 1 cycle/pixel |
| Max Polygons/Frame | ~4000 | Simple flat triangles |
| Max Textured Poly/Frame | ~1500-2000 | With texture, shading |
| Texture Cache | 2KB | Very limited |
| VRAM Bandwidth | ~100 MB/s | 16-bit accesses |
| Command FIFO | 12 entries | GPU command buffer |

---

## 9. RECOMMENDED OPTIMIZATIONS

### To Reduce Cycles

1. **Batch similar textures** - Reduces texture page changes
2. **Use triangles instead of quads** when possible
3. **Avoid semi-transparency** when not needed (costs 2x)
4. **Sort by depth** - Use depth sorting instead of Z-buffer
5. **Limit rendering area** - Use scissor test
6. **Pre-calculate Gouraud** when possible
7. **Sprite atlasing** - Group sprites in texture pages

### Typical Budgets for 60fps

| Scenario | Polygons | Frame Cycles | GPU % |
|----------|----------|--------------|-------|
| Simple scene | 500-1000 | ~400K | 45% |
| Medium scene | 1500-2500 | ~700K | 80% |
| Complex scene | 3000-4000 | ~850K | 95%+ |

---

## EMULATOR IMPLEMENTATION NOTES

For an accurate PS1 emulator, consider:

1. **Variable timing**: Actual cycles depend on VRAM contents
2. **Pipeline**: GPU can process commands while rendering
3. **DMA**: DMA transfers can happen in parallel
4. **Rasterizer**: The rasterizer is the main bottleneck
5. **FIFO**: Simulate the 12-entry command FIFO
6. **Precision**: PS1 GPU uses fixed-point arithmetic, not float

### Reference Timing
- 1 frame @ 60fps = ~887,000 GPU cycles available
- 1 scanline = ~3688 cycles
- HBlank = ~900 cycles
- VBlank = ~60 scanlines

