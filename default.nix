let
  unstable = import (fetchTarball https://nixos.org/channels/nixos-unstable/nixexprs.tar.xz) { };
  moz_overlay = import (builtins.fetchTarball https://github.com/mozilla/nixpkgs-mozilla/archive/master.tar.gz );
  nixpkgs = import <nixpkgs> { overlays = [ moz_overlay ]; };
  rustStableChannel = (nixpkgs.rustChannels.stable).rust.override {
    extensions = [
      "rust-src"
      "rust-analysis"
      "rustfmt-preview"
      "clippy-preview"
    ];
    targets = [
      "x86_64-unknown-linux-gnu"
    ];
  };
in
  with nixpkgs;
  stdenv.mkDerivation {
    name = "env";
    buildInputs = [
      rustStableChannel
      python3
      pkg-config
    ];
    nativeBuildInputs = [
      llvmPackages.libclang
      llvmPackages.libcxxClang
      clang
      espeak-ng
    ];
    LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
    # BINDGEN_EXTRA_CLANG_ARGS = "-isystem ${llvmPackages.libclang.lib}/lib/clang/${lib.getVersion clang}/include";
  }
