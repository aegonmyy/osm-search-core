{
  description = "osm_search_core — standalone OSM geocoding/search cdylib";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };
  outputs = inputs@{ nixpkgs, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        # Cross target triples supported by Logos
        targets = {
          "x86_64-linux" = { triple = "x86_64-unknown-linux-gnu"; ext = "so"; prefix = "lib"; };
          "aarch64-darwin" = { triple = "aarch64-apple-darwin"; ext = "dylib"; prefix = "lib"; };
          "x86_64-darwin" = { triple = "x86_64-apple-darwin"; ext = "dylib"; prefix = "lib"; };
          "x86_64-windows" = { triple = "x86_64-pc-windows-gnu"; ext = "dll"; prefix = ""; };
        };
      in {
        packages.${system}.default = pkgs.rustPlatform.buildRustPackage {
          pname = "osm-search-core";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          doCheck = false;
          # native build for current system
        };
      });
}
