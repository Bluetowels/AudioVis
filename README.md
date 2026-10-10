# AudioVis

A full-screen, GPU-driven audio visualiser for Windows, with experimental macOS and Android builds. It's built to respond as quickly as a fast spectrum analyser, and to show the shape of the music rather than just pulsing to the beat.

Every pixel stands for a pair of frequencies, and its colour shows how loud those two frequencies are together. Silence is black. Bass hits send a pulse out from the centre in a contrasting colour.

## What it does

- **Live system audio.** It listens to whatever the computer is playing (WASAPI loopback on Windows, a Core Audio tap on macOS). It can also capture one or more named output devices or an input such as a microphone.
- **Fast analysis.** The spectrum is variable-Q: 36 bins per octave across nine octaves from A0 (27.5 Hz). Every bin ends at the newest sample, so high notes appear immediately and nothing waits on the long bass windows.
- **Two views:**
  - **Cross**: x and y are both frequency, and a pixel lights when both of its frequencies are sounding. It can be mirrored into four quadrants and flipped on either axis.
  - **Circle**: frequency runs outwards as rings, with an optional angular pattern.
- **Bass effect.** A fast envelope (23 ms) of everything below the bass cutoff drives a pulse from the centre or a fill of the dark areas, and optionally a whole-picture flare.
- **Note controls.** Low notes can be sharpened to thin lines so a bass line is easy to follow, and the balance between speed and pitch detail is adjustable.
- **Stereo (optional).** Each sound can lean towards the side it's panned to. Wide, out-of-phase sound can be drawn in its own "surround" colour.
- **Experimental 3D.** Tilt the camera so loudness stands up as height, with orbit, a camera that flies around or into the picture, and a storm of raindrops.
- **HDR (Windows).** On an HDR display the loudest parts of the picture can be brighter than ordinary white, with adjustable base and peak brightness.
- **Palettes.** Ten built-in palettes: Ember, Ice, Aurora, Neon, Sunset, Mono, Rainbow, Zigzag, Candy and Contour. Any of them can be reversed or banded.
- **MIDI control.** It's mapped out of the box to a Korg nanoKONTROL2, and any slider can be re-assigned with MIDI learn.
- **Presets.** Save and load named snapshots of every slider and switch.

## How it works

AudioVis is written in Rust. Every frame it analyses the newest audio on the processor, hands the result to the graphics card as one small texture, and a shader draws the whole picture from it. The aim throughout is that what you see is what the music is actually doing: which notes are sounding, how loud, and when.

### A spectrum laid out like pitch

Most visualisers use a single FFT, which spaces its measurements evenly in hertz. Pitch doesn't work that way: each octave doubles the frequency. An evenly spaced spectrum therefore squeezes the bottom few octaves, where the bass line and most chords live, into a handful of points, and spends most of its detail on the top octave.

AudioVis uses a variable-Q transform instead. It measures 36 bins per octave, three per semitone, over nine octaves from A0 (27.5 Hz) to about 14 kHz: 324 bins, each as wide musically as every other. Single notes, chords and their harmonics stay separate from the bottom of the range to the top.

Each bin is measured over a stretch of audio suited to its own pitch, long for low notes and short for high ones. At full pitch detail a bin at A4 (440 Hz) looks at about 117 ms of sound; at the default setting, which favours speed, about 29 ms. The lowest notes would want well over a second, so they are capped (200 ms by default). Every one of these stretches ends at the newest sample, so a high note appears within a few milliseconds and nothing waits for the long bass measurements. "Speed vs pitch detail" and "Bass window" move this balance.

### Bass that arrives on time

Even capped, the bass bins of the spectrum are the slowest part of the picture, and a kick drum drawn 200 ms late looks wrong. So the bass effect doesn't use the spectrum at all. The waveform itself goes through a steep low-pass filter (fourth-order Butterworth) at the bass cutoff, and the loudness of what comes out is measured over the last 23 ms. The pulse from the centre follows that.

### Thin lines from smeared notes

A single note lights several neighbouring bins, and many more in the bass where the measurement has been shortened. The centre of that bump is still the note. Note sharpening finds each peak, fits a curve through it and its two neighbours to place the centre between bins, and redraws the bump as a line about one bin wide. Peaks that are only the skirt of a much louder neighbour are left out.

### Stereo

With Stereo on, left and right are analysed separately, keeping the timing of each bin as well as its size. Comparing the two gives, for every bin, where the sound sits between left and right, and how far the two channels are moving against each other, which is what makes sound wide and is what a surround upmix sends to the rear speakers. Both are averaged over about 150 ms and two semitones either side, so whole instruments lean together instead of flickering bin by bin.

### From levels to light

Auto-gain follows the loudest bin and falls back at a set speed when the music gets quieter, so quiet and loud tracks both fill the range of the palette. "Range" sets how far below that a sound can be and still show, "Decay" how long it lingers, and "Smoothing" how much neighbouring bins blur together. The palette then turns each level into a colour.

### Drawing

The graphics card receives one texture, 324 values wide: the level of every bin, with the stereo position and width when Stereo is on. A single full-screen shader works out every pixel from it, so the picture is as sharp at 4K as in a small window. The analysis takes well under a millisecond per frame on the PC it was developed on.

In 3D, the picture is first drawn into a square map and its brightness becomes height. The shader then looks across that landscape from the camera, which can orbit or fly through it. The storm adds up to 24,000 raindrops that fall onto the surface, with run-off streaking down the slopes and pooling in the dark, low ground.

<!-- source-only -->
### Implementation notes

- **Transform** (`src/analysis.rs`). Bins are grouped by the power-of-two FFT size that holds their window, and each group runs one FFT per frame ([rustfft](https://crates.io/crates/rustfft)). A bin's value is the product of that spectrum with a short precomputed kernel: the FFT of a Hann-windowed complex tone at the bin's frequency, right-aligned in the frame so its window ends at the newest sample. Only the few kernel points around the bin are kept.
- **Bass meter** (`src/analysis.rs`). Two second-order low-pass stages (Q 0.541 and 1.307) make the fourth-order Butterworth; a running sum of squares over 23.2 ms gives the level.
- **Sharpening** (`src/analysis.rs`). A peak is the highest bin within its own main lobe and within about 18 dB of the strongest bin nearby. Its position comes from a parabola through three points, and it is redrawn as a Gaussian with a sigma of 0.8 bins.
- **Rendering** (`src/render.rs`, `src/shader.wgsl`). Shaders are written in WGSL and run through [wgpu](https://wgpu.rs), natively on Vulkan (Windows) or Metal (macOS). The data texture also carries, for each bin, the loudest level within 2, 4, 8 ... 512 bins. The 3D view uses those rows to know how high the surface can be along a stretch of ground, so it can step across empty space without missing a thin wall.
- **Interface.** The panel is [egui](https://www.egui.rs) through eframe; audio capture is [cpal](https://crates.io/crates/cpal); MIDI is [midir](https://crates.io/crates/midir).
<!-- /source-only -->

## Requirements

- Windows 10 or 11, with a GPU that supports Vulkan (developed on an AMD Radeon RX 7900 XTX)
- Or, experimentally, macOS 14.6 or later on Apple silicon or Intel. See [macOS](#macos).
- Or, experimentally, Android 10 or later on a phone or tablet with Vulkan. See [Android](#android).
<!-- source-only -->
- To build from source: [Rust](https://rustup.rs) (stable, edition 2024) and the Visual Studio C++ Build Tools with a Windows SDK
<!-- /source-only -->

<!-- source-only -->
## Building

```powershell
cd visualiser
cargo build --release
```

The executable ends up in `target\release\audiovis.exe`, or in your `CARGO_TARGET_DIR` if you've set one. If the project folder is synced (OneDrive, for example), set `CARGO_TARGET_DIR` to somewhere outside it.

### Installer

```powershell
installer\build.ps1
```

This builds the release executable and packages it as `installer\output\AudioVis-Setup-<version>.exe`. It needs [Inno Setup 6](https://jrsoftware.org/isinfo.php) (`winget install JRSoftware.InnoSetup`), and on first run it downloads Microsoft's Visual C++ redistributable into `installer\redist\` to bundle with the setup. Neither folder is committed. The script needs PowerShell 7, and converts this README to an HTML page that the installer shows when it finishes.

### macOS app

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
installer/macos/build.sh
```

Run on a Mac, this builds a universal `installer/output/AudioVis.app` and packages it as `installer/output/AudioVis-<version>.dmg`. The same script runs on GitHub Actions (`.github/workflows/macos.yml`), which keeps the disk image as a downloadable artifact of each run.

### Android app

```bash
rustup target add aarch64-linux-android x86_64-linux-android
android/build-native.sh
gradle -p android assembleRelease
```

This needs the Android SDK and NDK (`ANDROID_HOME`, `ANDROID_NDK_HOME`), JDK 17 and Gradle 8.9 or later. The script builds the Rust library for phones and for the emulator, and Gradle packs it with a small Java layer (`android/app/src/main/java`) into `android/app/build/outputs/apk/release/app-release.apk`. The same steps run on GitHub Actions (`.github/workflows/android.yml`), which keeps the APK as a downloadable artifact of each run, then starts it in an emulator and keeps a screenshot.

The APK is signed with a throwaway key unless a keystore is supplied through `ANDROID_KEYSTORE_FILE`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD` (on GitHub, the secrets of the same names, with the keystore itself as `ANDROID_KEYSTORE_BASE64`). A build signed with a different key will not install over an older one: uninstall that first.
<!-- /source-only -->

## Running

<!-- installed-only: Start AudioVis from the Start menu, or from the desktop shortcut if you chose one. -->
<!-- source-only -->
```powershell
cargo run --release
```
<!-- /source-only -->

By default it captures the default output device. Play some music and it reacts.

### macOS

The macOS build is experimental: it is built and started automatically, but has not yet been tried on a real Mac.

- **Opening it the first time.** The app isn't signed with an Apple developer certificate, so macOS refuses to open a downloaded copy. Drag AudioVis to Applications, try to open it, then go to System Settings > Privacy & Security and choose Open Anyway.
- **Permission to listen.** The first time it captures sound, macOS asks whether AudioVis may record system audio (or use the microphone, for an input). Without that permission the picture stays black. It can be changed later in System Settings > Privacy & Security.
- **Output devices that also have inputs**, such as some USB audio interfaces, are captured from their input rather than from what they are playing.
- It draws with Metal rather than Vulkan, and settings are kept in `~/Library/Application Support/AudioVis/`.

### Android

The Android build is experimental: it is built automatically and started in an emulator, but has not yet been tried on a real phone. It needs Android 10 or later and a graphics chip with Vulkan. Copy the APK to the phone and open it; Android asks once whether to allow installing apps from that source.

It runs in landscape and fills the screen. A first run shows the built-in test signal, because every real source needs a permission. Choose a source under Audio source in the panel:

- **Other apps' sound.** Whatever other apps are playing. Android asks each time, with its "start recording or casting?" question, and shows a notification while the app is listening. Apps can refuse to be captured, and many music streaming apps do: if the picture stays black with music playing, that is why. Calls, alarms and notification sounds are never included.
- **Microphone.** The phone's microphone, without its noise reduction where the phone allows that.
- **Audio file.** Pick a file and the app plays it out loud, on repeat, and draws it. It stops while the app is out of sight.
- **Test signal.** The built-in kick, chord and hi-hat.

Touch stands in for the keyboard and mouse:

| Touch | Action |
| --- | --- |
| Tap the picture | Hide or show the panel |
| Swipe across the picture | Next or previous palette |
| Press and hold the picture | Palette menu |
| Press and hold a slider | MIDI learn, clear binding, reset |
| Double-tap a slider | Back to its default |

A USB MIDI controller plugged into the phone (with a USB OTG adapter if needed) is picked up within a couple of seconds, whichever make it is, and works as it does on the desktop. Settings are saved as they change. There are no command-line options, and descriptions on hover need a mouse.

### If the picture stays black

- **The music may be going to a different device.** With Voicemeeter or similar routing, programs often play to a device other than the Windows default. In the panel, set Audio source to "Chosen output devices" and tick the device carrying the music. The meters under the list show which devices are receiving sound. Several devices can be ticked and are added together; they must share a sample rate.
- **Only the first two channels of a device are used** (front left and right). A surround upmix further down the chain never reaches the visualiser.
- **Check the level settings.** With auto-gain off, the Reference level has to be near the level of the music.

### Keyboard and mouse

| Key | Action |
|---|---|
| Tab or F1 | Show or hide the settings panel |
| F11 | Toggle fullscreen (Esc leaves fullscreen) |
| F2 | Show or hide the FPS counter |
| F3 | Show or hide the FPS graph |
| F4 | Show or hide the picture of the controller |
| Right-click the picture | Palette menu |
| Double-click a slider | Return it to its default (shown by a small mark on the slider) |
| Right-click a slider | MIDI learn, clear the binding, or reset to default |

Rest the pointer on any setting to see a description of what it does; the "Descriptions on hover" button at the top of the panel turns these off. Click a section heading to fold that section away.

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
| `--underlay` | Start with the bass drawn as a fill of the dark areas |
| `--controller` | Start with the picture of the controller showing |
| `--fps` | Start with the FPS counter and graph showing (first run or self-test only) |
| `--hdr` | Ask for HDR output on this run (Windows only) |
| `--no-vsync` | Uncap the frame rate for this run |
| `--midi-port NAME` | Use a specific MIDI input port |
| `--set key=value` | Set any slider by its key, e.g. `--set bass_amount=2` |
| `--lyrics` | Show lyrics on this run (Windows only; see [Lyrics](#lyrics)) |
| `--lyrics-file song.lrc` | Show the lyrics in this LRC file, timed from when the app starts, instead of looking any up |

The app uses Vulkan (Metal on macOS) on the high-performance GPU by default. To override this, set the `WGPU_BACKEND` and `WGPU_POWER_PREF` environment variables.

<!-- source-only -->
For development there is a self-test: `--selftest file.png --seconds N` runs on default settings for N seconds, saves a screenshot, prints frame timings and exits. `--show-hint key` displays that slider's description in the screenshot.
<!-- /source-only -->

## Settings

Defaults are what a first run or "Reset everything to defaults" gives.

### Level

| Setting | Default | Range | What it does |
|---|---|---|---|
| Auto-gain | on | | Full brightness follows the loudest part of the music |
| Reference level | -12 dBFS | -60 to 12 | Level shown at full brightness when auto-gain is off |
| Auto-gain speed | 4 dB/s | 0.5 to 40 | How quickly the picture turns back up after a loud passage |
| Range | 40 dB | 20 to 90 | How far below full brightness still shows |
| Slope | 0 dB/oct | -3 to 3 | Tilts the balance about the middle of the spectrum: positive brings out treble, negative brings out bass |
| Contrast curve | 1.2 | 0.4 to 3 | How colour climbs from quiet to loud |
| Master brightness | 1.0 | 0 to 2 | Overall brightness |

### Notes and timing

| Setting | Default | Range | What it does |
|---|---|---|---|
| Bass note sharpening | 0.3 | 0 to 1 | Redraws each smeared low note as a thin line at its pitch |
| Bass window | 200 ms | 100 to 1000 | Audio the lowest notes are measured over; longer is sharper in pitch but slower |
| Note sharpening (whole spectrum) | 0.2 | 0 to 1 | Thins every note to a fine line; best on clean, pitched music |
| Speed vs pitch detail | 0.25 | 0.25 to 1 | Lower reacts faster and spreads notes across more bins |

### Bass boost

| Setting | Default | Range | What it does |
|---|---|---|---|
| Bass boost | on | | Bass hits drive an extra effect |
| Bass boost amount | 0.5 | 0 to 2 | Strength; above 1, big hits flood the frame |
| Bass cutoff | 150 Hz | 40 to 400 | Sound below this drives the effect |
| Bass release | 60 ms | 10 to 500 | How long the effect takes to die away |
| Bass glow | 0.3 | 0 to 1 | How far the pulse spreads, or how dim an area can be and still fill |
| Style | Pulse from the middle | or Fill dark areas | A glowing disc over the picture, or colour underneath it |
| Bass also flares the whole frame | off | | Bass hits also brighten the whole picture |
| Custom bass colour | off | | Off uses a colour chosen to contrast with the palette |

### Picture

| Setting | Default | Range | What it does |
|---|---|---|---|
| Shape | Cross | Cross, Circle | The two views |
| Mirror | Four quadrants | Off, Four quadrants, Left / right, Top / bottom | Cross view only |
| Flip x, Flip y | off | | Unticked, bass is in the middle; ticked puts treble there |
| Combine blend | 0 | 0 to 1 | 0 needs both of a pixel's frequencies; 1 lets one frequency light its row and column |
| Decay / persistence | 0 ms | 0 to 600 | How long a sound lingers after it stops |
| Smoothing between bins | 1.5 | 0 to 6 bins | Blurs neighbouring frequencies together |
| Lowest frequency | 27.5 Hz | up to 8 octaves higher | Bottom of the displayed range |
| Highest frequency | about 14 kHz | down to 55 Hz | Top of the displayed range |
| Circle: angular pattern | 0.5 | 0 to 1 | Runs a second frequency round the ring, like the cross bent into a circle |

### Stereo

| Setting | Default | Range | What it does |
|---|---|---|---|
| Stereo | on | | Draws each sound towards the side it is panned to |
| Stereo emphasis | 1.0 | 0 to 1 | How strongly sounds are pushed to their side |
| Surround colour amount | 1.0 | 0 to 1 | How strongly out-of-phase sound takes the surround colour |
| Custom surround colour | on, red (255, 0, 0) | | Off uses the palette's own surround colour |

Stereo position is averaged over about 150 ms and two semitones, so it's steadier than the rest of the picture and slightly slower.

### 3D (experimental)

| Setting | Default | Range | What it does |
|---|---|---|---|
| 3D tilt | 0 degrees | 0 to 70 | 0 is the flat picture; higher tips the camera back and loudness becomes height |
| 3D height | 0.6 | 0 to 1 | How tall the brightest parts stand |
| 3D orbit speed | 0 deg/s | -30 to 30 | Circles the camera round the centre |
| 3D flight speed | 0 | 0 to 2 | Above 0, the camera flies a wandering path; takes over from tilt and orbit |
| 3D flight depth | 0 | 0 to 1 | 0 flies above and around; 1 flies down among the peaks |
| 3D flight look ahead | 0 | 0 to 1 | 0 looks at the centre; 1 looks along the flight path |
| 3D storm | 0 | 0 to 1 | Raindrops that fall, splash on the surface and evaporate |
| Surface | Matte | Matte, Gloss, Metal, Glass | What the surface looks as if it is made of |
| Surface strength | 1.0 | 0 to 1 | How strongly the surface takes on that look; 0 is plain matte |

The 3D view is the heaviest part of the app. At 3840 x 2160, steep tilts or low flights over a dense picture can drop below the display's refresh rate.

### Colour

| Setting | Default | Range | What it does |
|---|---|---|---|
| Palette | Ember | 28 palettes | Colours from quiet to loud; 18 are smooth and 10 jump from colour to colour |
| Palette drift | 0 s (off) | 0 to 120 | Above 0, the colours blend on from palette to palette, spending this many seconds on each |
| Colours from the album cover | off | | Windows only: takes the palette from the cover of the track that is playing |
| Colour banding | 0 | 0 to 1 | 0 blends smoothly; 1 gives hard-edged bands |
| Reverse palette | off | | Swaps the palette end for end |
| Bloom | 0.3 | 0 to 1 | A soft glow that spreads from the bright parts of the picture; 0 is off |
| Background | Black | Black, Album cover | What shows where the picture is dark |
| Background brightness | 0.4 | 0 to 1 | How bright the cover is |
| HDR base brightness | 200 nits | 80 to 500 | HDR only: how bright ordinary parts of the picture and the panel are |
| HDR peak brightness | 1000 nits | 200 to 2000 | HDR only: how bright the very loudest parts go |
| HDR test pattern | off | | HDR only: white patches at 80, 200, 400, 800 and 1600 nits, plus the base and peak |

### Lyrics

Windows only.

| Setting | Default | Range | What it does |
|---|---|---|---|
| Show lyrics | off | | Shows the words of the song over the picture, in time with the music |
| Place | Bottom | Top, Middle, Bottom, Circle | Lines across the picture at that height, or round a circle about its middle |
| Lyrics size | 5% of the picture's height | 2 to 12 | Height of the line being sung |
| Lyrics strength | 0.4 | 0 to 1 | How much the words that aren't being sung show; low leaves them faint so the picture comes first |
| Show the next line | on | | The line to come, small and dim, under the one being sung |
| Lyrics sync offset | 0 ms | -1000 to 1000, in steps of 10 | Moves the lyrics earlier (negative) or later (positive); kept separately for each music app |

With Top, Middle or Bottom, the line being sung runs across the picture at that height. The line before fades out above it and the line to come waits below. Each line appears about 150 ms before it's sung.

With Circle, the lyrics run round a circle about the middle of the picture and scroll, each line passing the top while it's sung; earlier lines move away to the left and the lines to come arrive from the right. In 3D the words lie on the picture, on the far side of the circle from the camera, so they tilt, turn and fly with it; steep tilts make them small, and Lyrics size makes up for it. "Show the next line" has no effect there.

The words take their colours from the palette and are drawn faint, so they sit in the picture instead of over it. Where the lyrics have a time for every word, the word being sung comes forward: it swells, lifts, turns bright and glows, with the glow following the bass, then settles back as the next word starts. Most lyrics on LRCLIB only time whole lines, and those are shown a line at a time with no word picked out. In HDR the words stay at the base brightness.

**Where the lyrics come from.** The app reads the title, artist, album and length of what's playing from Windows' media controls (the same details the volume flyout shows), which Spotify, Tidal, browsers and most players fill in. It then looks the track up on [LRCLIB](https://lrclib.net), a free, crowd-sourced lyrics database. Only tracks that have time-synced lyrics there are shown, so some tracks have none, and the timings are only as good as whoever contributed them. The lyrics remain the copyright of their owners. They're fetched when a track plays, for display on your own screen, and nothing in this project redistributes them.

**Privacy.** "Show lyrics" is off by default. While it's on, the title, artist, album and length of each track you play are sent to lrclib.net. Nothing is sent while it's off. Results, including "nothing found", are kept in `%APPDATA%\AudioVis\lyrics\` so each track is only asked about once; a track with no lyrics is asked about again after a week.

**Staying in time.** Players report their position only now and then (at the start of a track and when you seek), so the app counts forward from the last report. If the lyrics run ahead of or behind what you hear, for example because of delay in your audio chain, move the sync offset.

The built-in font covers Latin, Greek and Cyrillic letters; lyrics in other scripts show as empty boxes.

**Colours from the album cover.** With this ticked, each track gets its own palette: the cover's two strongest colours, from dark to bright, with a bass colour chosen to stand apart from them. A black and white cover gives a grey picture with a red bass. Colours fade across when the track changes. With nothing playing, or a player that shows no cover, the chosen palette is used.

**Palette drift.** Smooth palettes drift through the other smooth ones in turn, and the abrupt ones (Rainbow, Zigzag, Candy, Contour, Harlequin, Circuit, Tropic, Glitch, Stained glass and Wasp) through each other, starting from the chosen palette. While a cover's colours are showing, they take over.

**Background.** Black keeps the rule that silence is black. Album cover is the playing track's cover, blurred and dim, showing only where the picture is dark.

**Surface.** In 3D, Gloss adds white highlights that slide over the ridges as the camera moves, Metal mirrors a bright sky in the surface's own colour, and Glass is dim face on and bright at its edges with sharp glints. Highlights only appear where there is sound.

The cover, title and artist used here and by the track card come from Windows' media controls on this PC. Nothing is sent anywhere for them; only lyrics use the network.

### Starfield

| Setting | Default | Range | What it does |
|---|---|---|---|
| Show stars | off | | A field of stars behind the picture, showing where it is dark |
| Star brightness | 0.4 | 0 to 1 | How bright the stars are |
| Star density | 0.5 | 0 to 1 | How many there are, from a sparse scatter to a crowded sky |
| Star size variety | 0.5 | 0 to 1 | 0 makes them all alike; at 1 most are small and a few are much larger and brighter |
| Star flight speed | 0 | 0 to 6 | Above 0, the view flies forward through the stars without end; gentle up to about 1, and at speed the stars draw out into streaks |
| Star flight bass | 0 | -1 to 1 | Above 0, each bass hit surges the flight forward; below 0, each hit holds it back |
| Star flight turn | 0 | 0 to 1 | Above 0, each bass hit throws the flight into a turn, a different way each time |
| Star flight turn hold | 0 s | 0 to 4 | How long a turn is held at its fullest before it straightens up |

The stars twinkle with the treble. Still, they drift slowly when the picture is flat, and in 3D they surround it and move with the camera. With Star flight speed above 0 the view flies forward through them: they come up out of the distance and rush past the edges. The stars are worked out as they are needed, not stored, so the flight never ends or loops. Star flight bass ties its speed to the same bass as the bass pulse, up to five times the speed on a hit at 1, or down to a standstill at -1. Star flight turn swings the point the stars stream from away from the middle on each hit, with the stars sliding across, and straightens up as the hit dies away. Stars and the album cover background can be on together.

### App

| Setting | Default |
|---|---|
| Audio source | Default output device |
| Window | 1280 x 720, panel shown |
| Vsync | on (a tick box at the top of the panel; a change applies when the app is restarted) |
| Descriptions on hover | on |
| HDR output | off (a tick box at the top of the panel; a change applies when the app is restarted) |
| FPS counter, FPS graph | off |
| Show lyrics | off |
| Show each track's title as it starts | on (Windows only: title, artist, album and cover in the top-right corner for a few seconds) |
| Fly the title in from the distance | off (instead of the corner, the cover and title start as a dot far ahead and come towards you, growing to fill the picture and thinning away as they do; with the stars flying they come from the point the stars stream out of) |

### HDR

HDR output needs an HDR display with HDR switched on in Windows. Tick "HDR output" at the top of the panel and restart the app, or start it with `--hdr`. Ordinary colours are then shown at the base brightness and the brightest colours of the palette climb towards the peak; the panel stays at the base brightness. If the display doesn't offer an HDR surface, the app carries on in standard range.

To get an HDR surface the project carries a copy of one of its UI components, `visualiser/vendor/egui-wgpu`, with a few small changes marked "AudioVis".

## MIDI (Korg nanoKONTROL2)

The app connects to the first MIDI input whose name contains "nanoKONTROL2". On the controller's factory mapping:

| Strip | Fader | Knob |
|---|---|---|
| 1 | Range | Contrast curve |
| 2 | Note sharpening (whole spectrum) | Speed vs pitch detail |
| 3 | Bass boost amount | Bass glow |
| 4 | Decay / persistence | Smoothing between bins |
| 5 | Combine blend | Highest frequency |
| 6 | Palette | Colour banding |
| 7 | 3D tilt | 3D height |
| 8 | 3D orbit speed | 3D flight speed |

- **S buttons 1 to 8:** flip x, flip y, mirror mode, stereo, auto-gain, bass boost, reverse palette, show or hide the panel
- **R button 1:** switch between cross and circle
- **Track arrows:** previous and next palette
- **Marker buttons:** SET turns lyrics on or off; < and > move the lyrics sync offset 10 ms earlier or later

**Controller picture.** F4, or the "Controller" button at the top of the panel, draws the nanoKONTROL2 across the bottom of the screen with every knob, fader and button labelled with what it does. White marks show where the hardware is and blue marks show where the setting is; lit buttons are switches that are on. Right-click any control on the picture to give it a different setting or job, or to unassign it.

Every other slider has no default control. To assign one, right-click the slider and choose MIDI learn, then move a fader or knob, or right-click a control on the controller picture. "Restore default controller mapping" puts the assignments above back.

Faders glide between steps, and frequency and time controls move logarithmically. After you load a preset or move a slider with the mouse, a fader takes over once you move it to the slider's position. Buttons are treated as toggles: every message from a button counts as one press.

## Presets and saved settings

Settings are saved automatically to `%APPDATA%\AudioVis\settings.json`, including the audio source, the controller mapping and which panel sections are folded. Presets are saved to `%APPDATA%\AudioVis\presets\` and hold every slider and switch, but not the audio source, the controller mapping, whether lyrics are shown or the lyrics sync offset. Neither is stored in this repository.

## Troubleshooting

- **A message box says it couldn't start the graphics card.** The app needs a GPU with Vulkan support and a current driver.
- **Nothing appears in a terminal.** The release build has no console window of its own. Started from a terminal it prints there (`--list-devices`, the self-test); started from a shortcut, errors are shown in a message box.
<!-- source-only -->
- **"Access is denied" when running a freshly built exe.** Windows Defender's attack surface reduction rules can block new unsigned programs. Add an exclusion for the build output folder.
- **The build fails with "failed to remove file audiovis.exe".** The app is still running; close it and build again.
<!-- /source-only -->
- **Tearing or stutter.** Check that nothing in the graphics driver is forcing vsync off, and use the FPS graph (F3) to see whether frames are being dropped.

<!-- source-only -->
## Project layout

```
visualiser/        Rust app (eframe/egui UI, wgpu rendering)
  src/lib.rs       app, settings panel, 3D camera, command-line options
  src/main.rs      the desktop program: starts the app in the library
  src/android.rs   Android only: sound and requests passed to and from the Java layer
  src/audio.rs     audio capture (system output, named devices, inputs)
  src/analysis.rs  variable-Q spectrum, note sharpening and bass meter
  src/params.rs    every adjustable setting, its description, and the palettes
  src/lyrics.rs    LRC lyrics parser, LRCLIB lookup and its cache
  src/nowplaying.rs  Windows only: what other apps are playing, from the media controls
  src/midi.rs      MIDI controller input and learn
  src/render.rs    GPU pipelines (picture, 3D map, raindrops)
  src/shader.wgsl  cross and circle views, 3D, rain, colour mapping
  examples/smtc_probe.rs  prints what the Windows media controls report, for checking a player
installer/         Inno Setup script and build script for the Windows installer
installer/macos/   build script and Info.plist for the macOS app and disk image
android/           Gradle project, Java layer and build script for the Android APK
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
<!-- /source-only -->
