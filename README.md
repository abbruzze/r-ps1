![workflow status](https://github.com/abbruzze/r-ps1/actions/workflows/build_and_release.yml/badge.svg)
[![Release](https://img.shields.io/github/v/release/abbruzze/r-ps1)](https://github.com/abbruzze/r-ps1/releases)
[![Language](https://img.shields.io/github/languages/top/abbruzze/r-ps1)]()
[![Downloads](https://img.shields.io/github/downloads/abbruzze/r-ps1/total)](https://github.com/abbruzze/r-ps1/releases/latest)

<div align="center">
  <img src="images/r-ps1_logo.png" alt="r-ps1 logo" width="614">
</div>

# R-PS1 ver 0.9.4
Rust Playstation 1 emulator

My first Rust development after years of Scala and OO projects, it was not a simple task but gave me a new perspective about software programming.

## Features
- CPU: full R3000A emulation, passes all [AmiDog's psxtest_cpu](https://psx.amidog.se/lib/exe/fetch.php?media=psx:download:psxtest_cpu.zip) tests
- GTE (Geometry Transformation Engine)
- GPU (Graphical Processing Unit), based on plain software rasterization algorithms (lines, rectangles, triangles)
  - timings emulation of GPU can be configured, but is a raw approximation bases on AI hypotesis
  - thanks to **fast_image** crate is it possible to configure the way the final image is scaled
- SPU (Sound Processing Unit), based on jsgroth's [CoffeePSX](https://github.com/jsgroth/CoffeePSX)
- DMA (Direct Memory Access)
  - timings should be improved
  - CPU/DMA bus arbitration should be improved
- MDEC (Motion Decoder)
- CD-ROM, most of the commands implemented
  - ADPCM
  - the timings emulation should be improved to increase the number of playable games
- Timers
- NTSC and PAL support
- Controllers
  - Digital
  - Sony Mouse
- USB Controller support
- Memory Cards
- Basic debugging (via CLI) support

## Bios
The emulator needs a BIOS image to run.
Unfortunately, the current implementation does not work with [OpenBIOS](https://pcsx-redux.consoledev.net/openbios/).
So you have to use one of the official BIOS images: below a list of official ones with auto region discovery based on MD5:

| Redump Name | Region | Date | MD5 |
|---|---|---|---|
| ps-10j | Japan | 1994-09-22 | `239665b1a3dade1b5a52c06338011044` |
| ps-11j | Japan | 1995-01-22 | `849515939161e62f6b866f6853006780` |
| ps-20a | USA | 1995-05-07 | `dc2b9bf8da62ec93e868cfd29f0d067d` |
| ps-20e | Europe | 1995-05-10 | `54847e693405ffeb0359c6287434cbef` |
| ps-21a | USA | 1995-07-17 | `da27e8b6dab242d8f91a9b25d80c63b8` |
| ps-21e | Europe | 1995-07-17 | `417b34706319da7cf001e76e40136c23` |
| ps-21j | Japan | 1995-07-17 | `cba733ceeff5aef5c32254f1d617fa62` |
| ps-22a | USA | 1995-12-04 | `924e392ed05558ffdb115408c263dccf` |
| ps-22e | Europe | 1995-12-04 | `e2110b8a2b97a8e0b857a45d32f7e187` |
| ps-22d | Japan | 1996-03-06 | `ca5cfc321f916756e3f0effbfaeba13b` |
| ps-22j | Japan | 1995-12-04 | `57a06303dfa9cf9351222dfcbb4a29d9` |
| ps-22j(v) | Japan | 1995-12-04 | `81328b966e6dcf7ea1e32e55e1c104bb` |
| ps-30a | USA | 1996-11-18 | `490f666e1afb15b7362b406ed1cea246` |
| ps-30e | Europe | 1997-01-06 | `32736f17079d0b2b7024407c39bd3050` |
| ps-30j | Japan | 1996-09-09 | `8dd7d5296a650fac7319bce665a6a53c` |
| ps-40j | Japan | 1997-08-18 | `8e4c14f567745eff2f0408c8129f72a6` |
| ps-41a(w) | USA | 1997-08-18 | `b84be139db3ee6cbd075630aa20a6553` |
| ps-41a | USA | 1997-12-16 | `1e68c231d0896b7eadcad1d7d8e76129` |
| ps-41e | Europe | 1997-12-16 | `b9d9a0286c33dc6b7237bb13cd46fdee` |
| psone-43j | Japan | 2000-03-11 | `8abc1b549a4a80954addc48ef02c4521` |
| psone-44a | USA | 2000-03-24 | `9a09ab7e49b422c007e6d54d7c49b965` |
| psone-44e | Europe | 2000-03-24 | `b10f5e0e3d9eb60e5159690680b1e774` |
| psone-45a | USA | 2000-05-25 | `6e3735ff4c7dc899ee98981385f6f3d0` |
| psone-45e | Europe | 2000-05-25 | `de93caec13d1a141a40a79f5c86168d6` |
| ps2-50j | Japan | 2000-10-27 | `d8f485717a5237285e4d7c5f881b7f32` |

## Usage
You can download last binaries from [Releases](https://github.com/abbruzze/r-ps1/releases) for Linux and Windows.
The emulator can be run on CLI as well. Below an example using --help on Windows:
```
Rust Playstation 1 emulator

Usage: r-ps1.exe [OPTIONS]

Options:
      --bios <FILE>
          Path to bios file

      --disc <FILE>
          Path to disc image file or EXE file

      --config <FILE>
          Path configuration file

      --region <REGION>
          Region

          Possible values:
          - usa:    America
          - europe: Europe
          - japan:  Japan
          - auto:   Automatic: the region will be the same of the disc

      --debugger
          Debugger enabled

      --log-level <LEVEL>
          Log level

          [possible values: error, warn, info, debug]

      --log-file <FILE>
          Log file

      --full-screen
          Full screen enabled

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```
On linux be sure to have the following dependencies installed:
- libasound2-dev 
- libudev-dev

If you are runinng it for the first time you must provide at least the bios option: the emulator will generate a default configuration file in the current directory.
Then you can just launch the emulator without any options, taking the necessary parameters from the configuration file.
Below an example of configuration file:
```yaml
disc_path: C:\temp\bloody roar 2.zip
bios_path: C:\temp\SCPH1001.BIN
region_policy: Auto
controllers:
  controller_1:
    controller_type: Digital
    controller_enabled: true
    controller_keymap:
      cross: KeyX
      circle: KeyZ
      square: KeyA
      triangle: KeyS
      l1: KeyQ
      l2: KeyW
      r1: KeyE
      r2: KeyR
      start: Enter
      select: ShiftRight
      dpad_up: ArrowUp
      dpad_down: ArrowDown
      dpad_left: ArrowLeft
      dpad_right: ArrowRight
    memory_card_path: C:\temp\memcard1.mcd
    attach_to_usb: true
  controller_2:
    controller_type: Digital
    controller_enabled: true
    controller_keymap:
      cross: KeyK
      circle: KeyJ
      square: KeyL
      triangle: KeyH
      l1: KeyU
      l2: KeyI
      r1: KeyO
      r2: KeyP
      start: Digit0
      select: Digit1
      dpad_up: KeyY
      dpad_down: KeyH
      dpad_left: KeyN
      dpad_right: KeyM
    memory_card_path: null
    attach_to_usb: true
  save_writings_to_disk: true
  auto_discover_usb_controllers: true
  usb_direction_resolution: 0.1
audio_config:
  buffer_capacity_in_millis: 10
tty_enabled: false
debugger_enabled: false
memory_config:
  cpu_write_queue_enabled: false
log_config:
  log_file: null
  log_severity: info
gpu_config:
  command_delay_enabled: false
  rendering_type: Bilinear
  start_full_screen: false
cdrom_config:
  show_cdrom_access: true
cheats_config:
  cheats_enabled: false
  cheats_codes:
    - 800244FC 1D86
    - 800244FE 0800
    - 80007618 8006
    - 8000761A 3C01
    - 8000761C 0ACC
    - 8000761E 8C21
    - 80007620 6300
    - 80007622 2403 
    - 80007624 0164 
    - 80007626 A423 
    - 80007628 0008
    - 8000762A 03E0
```

## Supported disc format
You can attach a [cue](https://en.wikipedia.org/wiki/Cue_sheet_(computing)) file or a zip file containing a cue file and all its bin references.
Alternatively, an [EXE](https://www.retroreversing.com/ps1-exe) file can be used to start the emulator.

## How to change disc
You can insert a new disc (removing the old one if present) just using the drag&drop function, dragging in a valid file format (cue or zip only).

## Cheat codes
You can configure under cheats_config node a list of [GameShark](https://gamegenie.com/cheats/gameshark/ps1/index.html) cheat codes.
The cheats codes must be manually applied when the game has been launched with F4 key.

## USB Controller
When you plug in a USB controller it will be attached to the first logical controller (#1 or #2) not already attached to an USB controller and with the auto_discover_usb_controllers property set to true.

## Mouse
if you set the property controller_type to Mouse you can omit the controller_keymap node, configuring the Sony Mouse.

## Logging
If the log_config.log_file property is not set (or set to null) the logging will be redirected to standard output.
The log_severity property can be set to debug, info or error.

## Key Bindings
Below the default key bindings for controller 1 and 2:

Button        | Key for #1 | Key for #2 |
--------------|------------|------------|
 Start         | Enter         | 0|
 Select        | Shift Right   | 1|
 Cross         | X             | K|
 Circle        | Z             | J|
 Triangle      | S             | H|
 Square        | A             | L|
 L1            | Q             | U|
 L2            | W             | I|
 R1            | E             | O|
 R2            | R             | P|
 Up            | Cursor Up     | Y|
 Down          | Cursor Down   | H|
 Left          | Cursor Left   | N|
 Right         | Cursor Right  | M|

Below the key bindings for GUI commands:

Button|Action
------|------
F1|Warp mode (maximum speed)
F2|VRAM view
F3|Mute sound
F4|Apply cheats codes if any
F10|Full screen mode (to exit from full screen mode use F10 or ESC key)
Space|Pause the emulation
Alt+F5|Reset the emulator
Shift+Alt+F5|Hard reset the emulator (will clear the memory as well)

## Memory card
The supported memory card format is **mcd** (128K binary format).
The release package contains a formatted empty card file you can rename and use to save games progress.
If the property save_writings_to_disk is set to true the emulator will save the memory card contents to disk when the application is closed.

## Debugger
The debugger can be enabled by setting the debugger_enabled property to true or using the --debugger option.
The debugger can be used to debug the emulation: the current implementation is very basic and it's textual only.
The commands can be entered directly from the console.
Below a list of the available commands:

| Command | Description                                                                                  |
|---------|----------------------------------------------------------------------------------------------|
| `<ENTER>` or `r` | Step: execute the next instruction and show disassembly. `r` will show CPU registers as well |
| `regs` | Show CPU registers (PC, LO, HI, R0-R31)                                                      |
| `cop0` | Show Coprocessor 0 registers                                                                 |
| `go` | Switch to Free Mode (no breakpoints) or Break Mode (with active breakpoints)                 |
| `log <level>` | Change log level (debug, info, error)                                                        |
| `rw <hex_addr> <length>` | Read memory as word (32-bit) from the specified address                                      |
| `rh <hex_addr> <length>` | Read memory as halfword (16-bit) from the specified address                                  |
| `rb <hex_addr> <length>` | Read memory as byte (8-bit) from the specified address, with ASCII dump                      |
| `break` or `b` | Without arguments: list all active breakpoints                                               |
| `break <add\|remove> x <hex_addr>` | Add/remove an execute breakpoint at the address                                              |
| `break <add\|remove> r <hex_addr>` | Add/remove a read breakpoint at the address                                                  |
| `break <add\|remove> w <hex_addr>` | Add/remove a write breakpoint at the address                                                 |
| `break <add\|remove> o <hex_opcode>` | Add/remove a breakpoint on a specific opcode                                                 |


## Gallery
<div align="center">
  <img src="/images/crash bandicoot 3.png" width="385" height="307"/>&nbsp;&nbsp;&nbsp;&nbsp;
  <img src="/images/medal of honor.png" width="385" height="307"/>
  <br/><br/>
  <img src="/images/gran turismo.png" width="385" height="307"/>&nbsp;&nbsp;&nbsp;&nbsp;
  <img src="/images/gran turismo 2.png" width="385" height="307"/>
  <br/><br/>
  <img src="/images/duke nukem.png" width="385" height="307"/>&nbsp;&nbsp;&nbsp;&nbsp;
  <img src="/images/jackie chan.png" width="385" height="307"/>
  <br/><br/>
  <img src="/images/need for speed 2.png" width="385" height="307"/>&nbsp;&nbsp;&nbsp;&nbsp;
  <img src="/images/need for speed 3.png" width="385" height="307"/>
  <br/><br/>
  <img src="/images/Castlevania Symphony of the Night.png" width="385" height="307"/>&nbsp;&nbsp;&nbsp;&nbsp;
  <img src="/images/crash team racing.png" width="385" height="307"/>
  <br/><br/>
  <img src="/images/tekken3.png" width="385" height="307"/>&nbsp;&nbsp;&nbsp;&nbsp;
  <img src="/images/tomb raider 2.png" width="385" height="307"/>
  <br/><br/>
  <img src="/images/apocalypse.png" width="385" height="307"/>&nbsp;&nbsp;&nbsp;&nbsp;
  <img src="/images/batman and robin.png" width="385" height="307"/>
  <br/><br/>
  <img src="/images/c-12 final resistance.png" width="385" height="307"/>&nbsp;&nbsp;&nbsp;&nbsp;
  <img src="/images/rayman.png" width="385" height="307"/>
  <br/><br/>
  <img src="/images/driver2.png" width="385" height="307"/>&nbsp;&nbsp;&nbsp;&nbsp;
  <img src="/images/pandemonium 2.png" width="385" height="307"/>
  <br/><br/>
  <img src="/images/resident evil 2.png" width="385" height="307"/>&nbsp;&nbsp;&nbsp;&nbsp;
  <img src="/images/skullmonkeys.png" width="385" height="307"/>
  <br/><br/>
  <img src="/images/syphon filter.png" width="385" height="307"/>&nbsp;&nbsp;&nbsp;&nbsp;
  <img src="/images/Tony Hawk's Pro Skater 2.png" width="385" height="307"/>
  <br/><br/>
  <img src="/images/toy story 2.png" width="385" height="307"/>&nbsp;&nbsp;&nbsp;&nbsp;
  <img src="/images/urban chaos.png" width="385" height="307"/>
  <br/><br/>
  <img src="/images/final doom.png" width="385" height="307"/>&nbsp;&nbsp;&nbsp;&nbsp;
  <img src="/images/mortal kombat 3.png" width="385" height="307"/>
</div>
