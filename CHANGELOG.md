# Changelog

## Unreleased

- **Lyrics (Windows).** The words of the song can be shown over the picture in time with the music: the line being sung in the middle, the line before fading out and the line to come below. The track is read from Windows' media controls and the lyrics come from LRCLIB. Off by default, because turning it on sends the title and artist of what you play to lrclib.net. The words take the palette's colours and stay faint while the word being sung swells and glows. They can sit at the top, middle or bottom, or run round a circle that follows the 3D view. Size, strength, next-line preview and a per-app sync offset are adjustable, and the controller's marker buttons turn lyrics on and off and nudge the offset.

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
