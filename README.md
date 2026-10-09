# AudioVis

A full-screen, GPU-driven audio visualiser for Windows. It's built to respond as quickly as a fast spectrum analyser, and to show the shape of the music rather than just pulsing to the beat.

Every pixel stands for a pair of frequencies, and its colour shows how loud those two frequencies are together. Silence is black. Bass hits flare across the whole picture.

## What it does

- **Live system audio.** It listens to whatever Windows is playing (WASAPI loopback). It can also capture one or more named output devices or an input such as a microphone.
- **Fast analysis.** The spectrum is variable-Q: 36 bins per octave across nine octaves from A0 (27.5 Hz). Every bin ends at the newest sample, so high notes appear immediately and nothing waits on the long bass windows.
- **Two views:**
  - **Cross**: x and y are both frequency, and a pixel lights when both of its frequencies are sounding. It can be mirrored into four quadrants and flipped on either axis.
  - **Circle**: frequency runs outwards as rings, with an optional angular pattern.
- **Bass effect.** A fast envelope (23 ms) of everything below the bass cutoff drives a pulse from the centre, a fill of the dark areas, or a whole-picture flare.
- **Stereo (optional).** Each sound can lean towards the side it's panned to. Wide, out-of-phase sound can be drawn in its own "surround" colour.
- **Experimental 3D.** Tilt the camera so loudness stands up as height, with orbit, a free-flying camera and a "storm" effect.
- **Palettes.** Ten built-in palettes: Ember, Ice, Aurora, Neon, Sunset, Mono, Rainbow, Zigzag, Candy and Contour. Any of them can be reversed or banded.
- **MIDI control.** It's mapped out of the box to a Korg nanoKONTROL2, and any control can be re-assigned with MIDI learn.
- **Presets.** Save and load named snapshots of every slider and switch.

## Requirements

- Windows 10 or 11
- A GPU with Vulkan support (developed on an AMD Radeon RX 7900 XTX)
- To build from source: [Rust](https://rustup.rs) (stable, edition 2024) and the Visual Studio C++ Build Tools

## Building

```powershell
cd visualiser
cargo build --release
```

The executable ends up in `target\release\audiovis.exe`, or in your `CARGO_TARGET_DIR` if you've set one.

## Running

```powershell
cargo run --release
```

By default it captures the default Windows output device. Play some music and it reacts.

### Keyboard and mouse

| Key | Action |
|---|---|
| Tab or F1 | Show or hide the settings panel |
| F11 | Toggle fullscreen (Esc leaves fullscreen) |
| F2 | Show or hide the FPS counter |
| F3 | Show or hide the frame-time graph |
| Right-click the picture | Palette menu |

Hover over any setting in the panel to see a description of what it does.

### Command-line options

| Option | Meaning |
|---|---|
| `--list-devices` | List the audio output and input devices, then exit |
| `--default-output` | Capture the default output device |
| `--outputs "Name A;Name B"` | Capture the named output devices, mixed together |
| `--test-signal` | Use a built-in test signal instead of real audio |
| `--fullscreen` | Start fullscreen |
| `--hide-panel` | Start with the settings panel hidden |
| `--stereo` | Start with stereo on |
| `--circle` | Start in the circle view |
| `--no-vsync` | Uncap the frame rate |
| `--midi-port NAME` | Use a specific MIDI input port |
| `--set key=value` | Set any slider by its key, e.g. `--set bass_amount=2` |

The app uses Vulkan on the high-performance GPU by default. To override this, set the `WGPU_BACKEND` and `WGPU_POWER_PREF` environment variables.

## MIDI (Korg nanoKONTROL2)

On the controller's factory mapping:

- **Faders 1 to 8:** slope, range, reference level, bass amount, decay, combine blend, lowest frequency, master brightness
- **Knobs 1 to 8:** bass cutoff, bass release, auto-gain speed, bass glow, contrast, smoothing, highest frequency, palette
- **Buttons:** switches such as mirror, flips, stereo and reverse palette (each switch's description in the panel names its button)

To re-assign a control, right-click any slider in the panel for MIDI learn. "Restore default controller mapping" puts the factory assignments back. After you load a preset, a fader takes over once you move it to the slider's position.

## Settings and presets

Settings are saved automatically to `%APPDATA%\AudioVis\settings.json`, and presets are saved to `%APPDATA%\AudioVis\presets\`. Neither is stored in this repository.

## Project layout

```
visualiser/        Rust app (eframe/egui UI, wgpu rendering)
  src/main.rs      app, settings panel, command-line options
  src/audio.rs     WASAPI capture (system output, named devices, inputs)
  src/analysis.rs  variable-Q spectrum and bass meter
  src/params.rs    every adjustable setting and the palettes
  src/midi.rs      MIDI controller input and learn
  src/render.rs    GPU pipeline
  src/shader.wgsl  cross and circle views, 3D, colour mapping
preview/           Python offline prototypes that render mp4 previews
```

### Python previews

These scripts are early prototypes used to try out ideas before the Rust app existed. They render an mp4 from an audio file and need numpy, scipy, moderngl and ffmpeg on your PATH.

```powershell
cd preview
python render2d.py --mode cross --bass --mirror --audio "path\to\track.mp3"
```

Output is written to `preview\out\`, which is not committed.

## Audio files

No music is included in this repository, and none should be committed. Use your own tracks for the previews.

## Status

This is a personal project in active development. The repository is private and shared by invitation only.
