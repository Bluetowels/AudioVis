#!/bin/bash
# Run with an emulator (or a phone) attached to adb: installs the APK found
# in the given folder, starts the app, waits for it to draw, and saves
#   emulator.png   what is on the screen
#   emulator.txt   the app's log and whether it is still running
# Fails if the app is not running at the end.
set -uo pipefail

apk="$(ls "$1"/*.apk | head -n 1)"
package=com.bluetowel.audiovis

adb install -r "$apk"
adb logcat -c
adb shell am start -W -n "$package/.MainActivity"
sleep 25
adb exec-out screencap -p > emulator.png

running="$(adb shell pidof "$package" | tr -d '\r')"
{
    echo "apk: $apk"
    echo "process id after 25 s: ${running:-none (the app has stopped)}"
    echo
    echo "--- log ---"
    adb logcat -d -s AudioVis:V AndroidRuntime:E DEBUG:V libc:F wgpu:V
} > emulator.txt
cat emulator.txt

[ -n "$running" ]
