# Changelog

## Unreleased

- **Lyrics, track card and album cover on Android.** The Android app now knows what is playing, so lyrics, the track card, colours from the cover and the cover background work there. For other apps' music Android requires notification access, which the Lyrics section of the panel leads to; the app's own audio file is identified from its tags, and can be given a `.lrc` file by hand. Lyrics are fetched with Android's own networking, so the app now asks for the internet permission; it is only used while "Show lyrics" is on. Windows behaviour is unchanged.

## 0.4.0 (2026-10-10)

Everything new here is for Windows unless it says otherwise; the macOS and Android builds gain the parts that don't depend on knowing what is playing.

- **Lyrics (Windows).** The words of the song over the picture, in time with the music. The track is read from Windows' media controls and time-synced lyrics come from LRCLIB. Off by default, because turning it on sends the title and artist of what you play to lrclib.net. They can sit at the top, middle or bottom, run round a circle that follows the 3D view, or roll away into the distance as a yellow crawl, optionally justified. The words take the palette's colours; sliders set how much they show and how much the line being sung stands out. Size, next-line preview and a sync offset kept per music app are adjustable.
- **Track card (Windows).** Title, artist, album and cover in the corner for a few seconds as each track starts, or flown in from the distance. F5, or the controller's PLAY button, flies it in on demand.
- **Colours from the album cover (Windows).** The palette can be taken from the cover of the track that is playing.
- **More palettes, all systems.** Eighteen new ones, six of them abrupt like Candy and Zigzag, making 28. Palette drift blends slowly from one palette to the next.
- **Bloom, all systems.** A soft glow round the bright parts of the picture, on by default at a low setting.
- **Starfield, all systems.** Stars behind the picture that twinkle with the treble and move with the 3D camera, with settings for brightness, density and variety of size. They can be flown through without end, at anything from a drift to streaks, and the flight can surge, slow or turn with the bass. Off by default.
- **Album cover background (Windows).** The playing track's cover, blurred and dim, behind the picture.
- **3D surfaces, all systems.** Gloss, metal and glass, with highlights that move as the camera does, and a strength slider.
- **Controller.** The marker buttons turn lyrics on and off and nudge the sync offset.
## 0.3.0 (2026-10-09)

- **macOS (experimental).** The app builds for macOS 14.6 or later as a universal app in a disk image. It draws with Metal and captures system audio with a Core Audio tap. Windows behaviour is unchanged.
- **Android (experimental).** The app builds as an APK for Android 10 or later. It draws with Vulkan, takes its sound from other apps (where they allow it), the microphone or an audio file, is worked by touch, and picks up a USB MIDI controller. Windows behaviour is unchanged.
- **Slope no longer blacks out the picture.** It now tilts about the middle of the spectrum over a range of -3 to 3 dB per octave, and faint hiss can no longer take over auto-gain.
- **HDR output on Windows.** On an HDR display the picture can use brightness above ordinary white, with base and peak brightness sliders and a test pattern. Off by default; tick "HDR output" or start with `--hdr`.

## 0.2.0 (2026-10-09)

The first release.

- **Views.** Cross (frequency against frequency, mirrored into four quadrants, with flips) and circle (frequency as rings, with an optional angular pattern).
- **Audio.** Captures the default Windows output, one or more chosen output devices added together, or an input device. Only the front left and right channels are used.
- **Analysis.** Variable-Q spectrum, 36 bins per octave over nine octaves, with bass note sharpening, whole-spectrum note sharpening, and a speed versus pitch detail control.
- **Bass.** A fast bass envelope drives a pulse from the centre or a fill of the dark areas, with an optional whole-picture flare and a choice of colour.
- **Stereo.** Sounds lean towards the side they are panned to, and out-of-phase sound takes a surround colour.
- **3D (experimental).** Tilt, height, orbit, a camera that flies around or into the picture, and a storm of raindrops.
- **Colour.** Ten palettes, reversible, with adjustable banding.
- **Controller.** Korg nanoKONTROL2 support with MIDI learn, soft pickup, and an on-screen picture of the controller (F4) showing what every control does.
- **Panel.** Folding sections, descriptions on hover, default markers on sliders, double-click a slider to return to its default, presets, FPS counter and graph, Vsync option.
- **Installer.** A Windows installer built with Inno Setup.
