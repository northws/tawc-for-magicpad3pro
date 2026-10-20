# ARLinux (tawcroot-based) graphics and desktop stack

Notes on [arlinux.com](https://arlinux.com/) (project site), the
tawcroot-based Android app that runs full Linux distributions and desktops
on Qualcomm devices. Useful as a reference for finishing tawc's own
`libhybris-zink` presentation path.

## Repositories

| Repository | Role |
|-----------|------|
| [taowen/arlinux-rootfs](https://github.com/taowen/arlinux-rootfs) | build framework: component recipes, CI entry points, rootfs assembly |
| [taowen/mesa](https://github.com/taowen/mesa) | Mesa fork, branch `arlinux-official-base`, adds `-Darlinux-wsi` |
| [taowen/libhybris](https://github.com/taowen/libhybris) | libhybris fork, branch `arlinux` |
| [Halium/android-headers](https://github.com/Halium/android-headers) | Android headers used by the libhybris build |
| `taowen/arlinux-arch`, `-debian`, `-omarchy` | distribution submodules |

Builds run in GitHub Actions (`ubuntu-24.04-arm`) as "component capsules"
(`tools/build/components/*.json`, `tools/build/github-component.py`); the
distributions and desktop environments are separate components:
`arch`, `debian`, `guest-tools`, `linux-hybris`, `linux-mesa`, `lxqt`,
`omarchy`, `plasma`, `vulkan-capture`, `yibu`. Wine has its own pipeline
(`arlinux-wine-component.yml`). `tools/build/desktop-pipeline.py` assembles
the rootfs products and accepts `--distribution yibu|debian|arch|omarchy|
plasma|lxqt`.

## Mesa / GPU stack

From `tools/build/linux-gpu.sh`:

```sh
meson setup ... \
  -Dauto_features=disabled -Dgallium-drivers=zink \
  -Dvulkan-drivers=freedreno -Dfreedreno-kmds=kgsl \
  -Dplatforms=x11,wayland -Darlinux-wsi=true \
  -Degl-native-platform=auto -Degl=enabled -Dgles1=disabled \
  -Dgles2=enabled -Dopengl=true -Dglx=dri -Dgbm=enabled -Dglvnd=enabled
```

Key differences from tawc's current `libhybris-zink` build:

1. **Vulkan driver is Turnip, not the Android vendor driver.**
   `vulkan-drivers=freedreno` + `freedreno-kmds=kgsl` compiles Mesa's
   open-source Adreno Vulkan driver and reaches the GPU through
   `/dev/kgsl-3d0`. The Android vendor `libvulkan.so` (and libhybris'
   `ANativeWindow`-based WSI wrapper around it) is not in the path at all.
   This is why they do not hit the "zink renders, vendor swapchain images
   are empty" failure described in [libhybris-zink.md](libhybris-zink.md).
2. **`-Darlinux-wsi=true`**: their own Vulkan WSI, implemented in
   `src/vulkan/wsi/wsi_common_arlinux*.c` (option description: "KGSL
   presentation through TAWC-DRI and android_wlegl").
   - allocates Qualcomm gralloc buffers (usage `0x333` =
     HW_RENDER | HW_TEXTURE | SW_READ/WRITE_OFTEN),
   - imports them as Vulkan images for Turnip,
   - presents them over Wayland with `wl_surface_attach` +
     `wl_surface_damage` + `wl_surface_commit` and a `wl_surface_frame`
     callback (`wsi_common_arlinux_wayland.c`), i.e. the client-allocated
     `android_wlegl` buffer path tawc's compositor already implements.
3. **`glx=dri` + `gbm` + `glvnd` + `platforms=x11,wayland`.** `glx=dri`
   makes Mesa build `gallium/targets/dril` (the DRI megadriver that
   `zink_dri.so` points at) — the same module tawc has to patch in when it
   builds with `glx=disabled` (see `07-tawc-zink-wayland-fixes.patch`).
   X11 support covers Xwayland clients.

## libhybris

`tools/build/linux-gpu.sh` configures it with:

```sh
--with-default-hybris-ld-library-path=/vendor/lib64/egl:/vendor/lib64/hw:/vendor/lib64:/system/lib64:/system_ext/lib64
```

so vendor libraries win, unlike tawc's runtime workaround
(`HYBRIS_LD_LIBRARY_PATH=/system/lib64:/vendor/lib64`) for the
`__hash_memory` / vendor-libc++ problem. Their branch also carries an
`arlinux/graphics-protocols` tree: the `android_wlegl` and related
protocols used by the Mesa WSI and the compositor.

## Why this matters for tawc

- `/dev/kgsl-3d0` is world-writable on tawc's target device (Honor
  MagicPad 3 Pro, Adreno 840) and opens from the app uid, so the
  Turnip + KGSL route is viable inside tawc's sandbox.
- Adopting `arlinux-wsi` (plus `-Dvulkan-drivers=freedreno
  -Dfreedreno-kmds=kgsl`) would replace the vendor-driver WSI that currently
  produces empty swapchain images for zink, and would likely fix on-screen
  desktop GL for good.
- Until then, tawc's `LIBHYBRIS` (GLES) backend already gives hardware
  rendering and on-screen presentation through the vendor driver.
