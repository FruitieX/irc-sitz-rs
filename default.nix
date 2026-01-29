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
  pkgs.llvmPackages.stdenv.mkDerivation {
    name = "env";
    buildInputs = [
      rustStableChannel
      python3
      pkg-config
      #llvmPackages.libclang
      #clang
      espeak-ng
    ];
    LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
    LD_LIBRARY_PATH = "${espeak-ng}/lib";
  }
