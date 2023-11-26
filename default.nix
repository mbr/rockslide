let
  rust-overlay = (import (builtins.fetchTarball
    "https://github.com/oxalica/rust-overlay/archive/7c94410d52d4e8bd72803fc1fe6c51fe179edaf5.tar.gz"));
in { pkgs ? (import <nixpkgs>) { overlays = [ rust-overlay ]; } }:
let
  stable-rust = pkgs.rust-bin.stable.latest.default.override {
    extensions = [ "rust-src" ];
  };
  rustPlatform = pkgs.makeRustPlatform {
    cargo = stable-rust;
    rustc = stable-rust;
  };
  cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
in rustPlatform.buildRustPackage {
  name = cargoToml.package.name;
  version = cargoToml.package.version;
  src = pkgs.lib.cleanSourceWith {
    filter = name: type: baseNameOf name != "docker.nix";
    src = (pkgs.lib.cleanSource ./.);
  };
  cargoLock = { lockFile = ./Cargo.lock; };
}
