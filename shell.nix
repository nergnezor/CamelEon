# NixOS dev shell: libraries that winit and wgpu load at runtime, plus udev
# (linked by gilrs for gamepads) and ALSA (sound, via cpal).
{ pkgs ? import <nixpkgs> { } }:
let
  runtimeLibs = with pkgs; [
    wayland
    libxkbcommon
    vulkan-loader
    libGL
    libx11
    libxcursor
    libxi
    libxrandr
    udev
    alsa-lib
  ];
in
pkgs.mkShell {
  nativeBuildInputs = [ pkgs.pkg-config ];
  buildInputs = runtimeLibs;
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
  # Tools installed with `cargo install` (dx for hot reload, wasm-bindgen).
  shellHook = ''
    export PATH="$HOME/.cargo/bin:$PATH"
  '';
}
