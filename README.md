# Tess's Android Wayland Compositor
TAWC runs CLI and graphical Linux programs on Android without root. Graphical apps get hardware acceleration with the phone's native graphics stack. The project consists of tawcroot (a performant alternative to PRoot), a Wayland compositor, and the UI to put it all together.

## This fork (MagicPad 3 Pro)

Fork of upstream TAWC with two additions aimed at the Honor MagicPad 3 Pro
(a Chinese tablet with a detachable keyboard/touchpad):

- **Touchpad two-finger scrolling.** The compositor accepts
  `SOURCE_TOUCHPAD` (upstream only handled `SOURCE_MOUSE`), captures the
  relative pointer while a touchpad gesture is active, and draws a cursor
  overlay, so clients such as Firefox scroll properly from the touchpad.
- **Physical-keyboard Chinese input.** The compositor implements its side of
  `zwp_input_method_v2` (plus `zwp_virtual_keyboard_v1`), so `fcitx5`
  running inside the distro can compose for Wayland clients. The candidate
  window is rendered above all windows, anchored just below the text caret.
  See [notes/text-input.md](notes/text-input.md) for the design.

### Using fcitx5 inside the distro

```bash
sudo pacman -S fcitx5 fcitx5-chinese-addons

mkdir -p ~/.config/fcitx5
cat > ~/.config/fcitx5/profile <<'PROFILE'
[Groups/0]
Name=Default
Default Layout=us
DefaultIM=pinyin

[Groups/0/Items/0]
Name=keyboard-us
Layout=

[Groups/0/Items/1]
Name=pinyin
Layout=

[GroupOrder]
0=Default
PROFILE

fcitx5 -d
```

`fcitx5-remote -n` prints the active input method and
`fcitx5-remote -s pinyin` switches to pinyin explicitly. Composition
happens inside the container; only the focused client receives the result.

## Features
- The app embeds Termux's widget for a familiar terminal UI (the Termux app itself is not required)
- When graphical apps are installed they can be run from TAWC's launcher menu
- Linux apps can be added to the phone's home screen and are presented alongside Android apps in the app switcher
- XWayland is included and wired up for hardware accelerated X11 support
- A built-in task manager lets you view and kill running Linux processes
- The `ando` command and storage binds allow Linux programs to interact with Android data if desired

## High-level design
- A stock Linux distro, such as Arch Linux ARM or Debian, is downloaded and extracted
- Linux programs, including the distro's native package manager, are run "inside" the distro's rootfs using `tawcroot`
- `tawcroot` emulates chroot and other syscalls to overcome the limitations of rootless Android (it's similar to PRoot but faster because it uses a single process)
- A Smithay-based Wayland compositor provides android integrations (like passing input from your normal Android keyboard)
- `libhybris` allows glibc Linux programs to load the standard Android graphics drivers
- Upstream `libhybris` doesn't work on stock Android, but [our fork](https://github.com/wmww/libhybris) does

## Limitations
- No real sandboxing of Linux apps beyond Android's own mechanisms. Due to its single-process design, tawcroot can be escaped
- No desktop GL support, only the graphics APIs provided by the phone (generally GLES and Vulkan)
- (Related) has not yet been tested or optimized for games
- Perf is better than alternatives, but not native
- Only arm64 official builds for now. Can be built for x86 but our libhybris tricks rely on arm.
- Requires Android 10+
- Android 12+ may kill rootfs processes past ~32 (the phantom process killer); heavy parallel jobs can fail. On Android 14+, enable Developer options → "Disable child process restrictions"

See [AGENTS.md](AGENTS.md) for more details.

## Building this fork

The toolchain is the same as upstream: JDK 21, the Android SDK with
platform 36, build-tools 36 and NDK 27.2.12479018, and Rust with
`cargo-ndk`. See [notes/building.md](notes/building.md) for the full list.

Debug APK (the default build, installed as `me.phie.tawc.dev` next to the
release app):

```bash
scripts/build-app.sh --abi=arm64-v8a --no-mesa
# -> app/build/outputs/apk/debug/app-debug.apk
```

Debug APK with the plain package ID (`me.phie.tawc`). This is what the
fork's release builds use; it can only be installed where no app signed
with upstream's key is installed, because the signatures differ:

```bash
TAWC_PACKAGE=me.phie.tawc scripts/build-app.sh --abi=arm64-v8a --no-mesa
```

Signed release APK. Release builds use the `libhybris,cpu` graphics set by
default and need a keystore you control. The keystore below is the fork's
own; losing it means future releases cannot update installed copies:

```bash
KEYSTORE_PATH=~/Android/keystore.jks \
KEYSTORE_PASS=... KEY_ALIAS=tawc-fork \
scripts/build-release-apk.sh
# -> app/build/outputs/apk/release/tawc-v<version>.apk
```

## Contributing
Issues are preferred over PRs. I welcome bug reports and feature requests, but no guarantee they can be handled on any particular timeline. If your issue has LLM-written content, please clearly mark it (ideally with what LLM wrote it), and always include a human-written description. Please include your app version, phone, Android version and distro (if relevant).

<details>
<summary>TL;DR, good:</summary>

> ## Issue title: program foo crashes
>
> I'm using Arch on TAWC v1 on my Pixel 10 running stock Android 17.
>
> I installed foo v1.2 with pacman but it crashes on launch. I asked fable about the problem, and it said:
>
> [CLAUDESLOP]

</details>

<details>
<summary>bad:</summary>

> ## PR title: [CLAUDESLOP]
>
> [CLAUDESLOP]

Code:
```
UNEXPLAINED CLAUDESLOP
```
</details>

## Licensing
All code outside of `deps/` is MIT ([LICENSE.MIT](LICENSE.MIT)). There's some GPLv3 code in the vendored dependencies (termux-shared's extra-keys widget), so the project as a whole is GPLv3 ([LICENSE](LICENSE)).

Per-component attribution for everything bundled in the app is in-app under Settings → About → Licenses.
